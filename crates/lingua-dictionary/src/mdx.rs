//! MDX（MDict）词典解析器 —— 只读、流式产出词条。
//!
//! 布局参考社区权威实现（mdict-js / mdict-analysis，基于官方
//! fileformat.md 分析），实测对齐 v1.2 引擎产物：
//!
//! ```text
//! [header]  u32 BE 长度 + UTF-16LE 属性文本 + 4 字节 adler32（所有版本）
//! [key 块区] 摘要(块数/词条数[/v2 信息区解压长]/信息区压缩长/数据区长[/v2 校验])
//!            信息区 = 逐块(块内词条数 + 首词 + 尾词 + 压缩长 + 解压长)
//!            数据区 = 逐块，块内逐词条(记录偏移 + NUL 结尾词头)
//! [record 块区] 摘要(块数/词条数/信息区长/数据区长)
//!               信息区 = 逐块(压缩长 + 解压长)
//! ```
//!
//! 版本差异：v1.2 数字是 u32 BE、文本长度前缀是 u8；v2.0 数字占
//! 8 字节槽（取低 4 字节）、文本长度前缀是 u16 BE、摘要多一个
//! 信息区解压长与校验和。压缩块（key 信息区 / key 块 / record 块）
//! 前有 `[类型 4B][adler32 4B]` 前缀：类型 0 = 未压缩，2 = zlib；
//! v1.2 未压缩块无前缀。
//!
//! **加密**：`Encrypted` bit 1（值 2，多数商业词库）只加密 key 信息区，
//! 密钥由区块自身字节 + 固定常数 0x3695 经 RIPEMD-128 派生，离线可解
//! （见 [`decrypt_key_block_info`]）；bit 0（值 1）绑定用户注册信息，
//! 离线无法解密，直接报错。

use encoding_rs::{BIG5, Encoding, GBK, UTF_16LE};
use flate2::read::ZlibDecoder;
use lingua_core::{AppError, AppResult, DictionaryDefinition, DictionaryEntry, Language};
use ripemd::Digest as _;
use std::io::Read as _;
use std::path::Path;

/// 解析后的 MDX 文件：词头索引常驻，释义按 record 块流式解压。
pub struct MdxFile {
    v2: bool,
    encoding: &'static Encoding,
    utf16: bool,
    title: Option<String>,
    /// 按 record 偏移升序排列的词条组（同偏移的多个词头合并为
    /// 首词头 + 词形变化别名）。
    groups: Vec<KeyGroup>,
    /// record 块布局：文件内偏移与解压后流偏移（用于按偏移切块）。
    record_blocks: Vec<RecordBlockLayout>,
    file: Vec<u8>,
}

struct KeyGroup {
    lemma: String,
    /// 指向同一释义记录的其余词头（别名 / 词形变化）。
    inflections: Vec<String>,
    /// 该释义在解压后 record 流中的起始偏移。
    record_offset: u64,
}

struct RecordBlockLayout {
    /// 压缩数据在文件中的起始偏移。
    file_offset: usize,
    comp_len: usize,
    /// 解压后数据在虚拟 record 流中的起始偏移。
    stream_offset: u64,
    decomp_len: usize,
}

/// 一个词条组：词头 + 别名 + 原始释义文本（HTML）。
pub struct MdxEntry {
    pub lemma: String,
    pub inflections: Vec<String>,
    pub text: String,
}

impl MdxEntry {
    /// `@@@LINK=` 重定向条目：MDict 里指向另一个词头的链接，
    /// 纯文本导入下没有意义，跳过。
    pub fn is_redirect(&self) -> bool {
        self.text.trim_start().to_lowercase().starts_with("@@@link=")
    }

    /// 转换为统一词典模型：纯文本 + 净化后的富文本 HTML 各存一份。
    pub fn to_dictionary_entry(
        &self,
        language: &Language,
        gloss_language: &Language,
        source: &str,
    ) -> DictionaryEntry {
        DictionaryEntry {
            lemma: self.lemma.clone(),
            language: language.clone(),
            gloss_language: gloss_language.clone(),
            pronunciations: Vec::new(),
            inflections: self.inflections.clone(),
            source: source.to_string(),
            definitions: vec![DictionaryDefinition {
                part_of_speech: None,
                definition: html_to_plain_text(&self.text),
                html: Some(crate::html_sanitize::sanitize_definition_html(
                    &self.text, source,
                )),
                examples: Vec::new(),
                synonyms: Vec::new(),
                antonyms: Vec::new(),
            }],
        }
    }
}

fn invalid(message: impl Into<String>) -> AppError {
    AppError::InvalidInput(message.into())
}

/// 解压一个 MDict 数据块：`[类型 4B][adler32 4B]` 前缀（类型 1 = LZO、
/// 2 = zlib）后是压缩流；类型 0 表示未压缩——v1.2 数据原样，
/// v2.0 仍带 8 字节前缀需跳过。
fn decompress_block(
    data: &[u8],
    v2: bool,
    expected_len: usize,
    context: &str,
) -> AppResult<Vec<u8>> {
    let first = data.first().copied().unwrap_or(0xff);
    match first {
        0x00 if !v2 => Ok(data.to_vec()),
        0x00 => Ok(data.get(8..).unwrap_or(data).to_vec()),
        0x01 => lzo(&data[8.min(data.len())..], expected_len, context),
        0x78 => zlib(data, expected_len, context),
        _ => zlib(&data[8.min(data.len())..], expected_len, context),
    }
}

fn lzo(data: &[u8], expected_len: usize, context: &str) -> AppResult<Vec<u8>> {
    let mut out = vec![0u8; expected_len];
    lzokay::decompress::decompress(data, &mut out)
        .map(|written| {
            out.truncate(written);
            out
        })
        .map_err(|error| invalid(format!("MDX 数据损坏（{context} LZO 解压失败）：{error}")))
}

/// MDict「bit 2 加密」的异或链（readmdict `_fast_decrypt` 的移植）：
/// 每字节 = 半字节交换 ^ 前一密文字节 ^ 序号 ^ 密钥轮转，首字节前值 0x36。
fn fast_decrypt(data: &mut [u8], key: &[u8; 16]) {
    let mut previous = 0x36u8;
    for (index, byte) in data.iter_mut().enumerate() {
        let original = *byte;
        *byte = original.rotate_left(4) ^ previous ^ (index as u8) ^ key[index % key.len()];
        previous = original;
    }
}

/// 解密「bit 2 加密」的 key 信息区（就地，仅 `[8..]` 段）：
/// 密钥 = `ripemd128(信息区[4..8] + 0x3695u32 LE)` 派生的 16 字节，
/// 完全离线可解，与用户注册信息无关。
fn decrypt_key_block_info(data: &mut [u8]) {
    let mut seed = [0u8; 8];
    seed[0..4].copy_from_slice(&data[4..8]);
    seed[4..8].copy_from_slice(&0x3695u32.to_le_bytes());
    let key = ripemd::Ripemd128::digest(seed);
    let split = 8.min(data.len());
    fast_decrypt(&mut data[split..], key.as_ref());
}

fn zlib(data: &[u8], expected_len: usize, context: &str) -> AppResult<Vec<u8>> {
    let mut out = Vec::with_capacity(expected_len);
    ZlibDecoder::new(data)
        .read_to_end(&mut out)
        .map_err(|error| invalid(format!("MDX 数据损坏（{context} 解压失败）：{error}")))?;
    Ok(out)
}

/// 顺序字节读取器：越界一律报错，不做静默截断。
struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    fn remaining(&self) -> usize {
        self.buf.len() - self.pos
    }

    fn take(&mut self, len: usize) -> AppResult<&'a [u8]> {
        if self.remaining() < len {
            return Err(invalid("MDX 数据损坏（意外结束）"));
        }
        let start = self.pos;
        self.pos += len;
        Ok(&self.buf[start..start + len])
    }

    fn skip(&mut self, len: usize) -> AppResult<()> {
        self.take(len).map(|_| ())
    }

    fn u8(&mut self) -> AppResult<u8> {
        Ok(self.take(1)?[0])
    }

    fn u16_be(&mut self) -> AppResult<u16> {
        let bytes = self.take(2)?;
        Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
    }

    fn u32_be(&mut self) -> AppResult<u32> {
        let bytes = self.take(4)?;
        Ok(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    /// MDict 数字：v1.2 为 u32 BE；v2.0 占 8 字节槽（跳过高 4 字节）。
    fn number(&mut self, v2: bool) -> AppResult<u64> {
        if v2 {
            self.skip(4)?;
            self.u32_be().map(u64::from)
        } else {
            self.u32_be().map(u64::from)
        }
    }

    /// MDict 文本长度前缀：v1.2 为 u8，v2.0 为 u16 BE（单位是字符，
    /// UTF-16 文件实际字节数 = 值 × 2）。
    fn text_len_prefix(&mut self, v2: bool) -> AppResult<usize> {
        if v2 {
            self.u16_be().map(|n| n as usize)
        } else {
            self.u8().map(|n| n as usize)
        }
    }

    /// 读一段「长度前缀 + 文本」（key 块信息区的首词/尾词用）。
    /// v2.0 文本后另有与字符等宽的填充尾字节。
    fn sized_text(
        &mut self,
        v2: bool,
        utf16: bool,
        encoding: &'static Encoding,
    ) -> AppResult<String> {
        let chars = self.text_len_prefix(v2)?;
        let bytes = self.take(chars * if utf16 { 2 } else { 1 })?;
        if v2 {
            self.skip(if utf16 { 2 } else { 1 })?;
        }
        Ok(decode_text(bytes, utf16, encoding))
    }

    /// 读一段 NUL 结尾文本（key 块内的词条词头）：按编码的字符宽度
    /// 扫描到终止符，终止符一并消费。
    fn nul_text(&mut self, utf16: bool, encoding: &'static Encoding) -> AppResult<String> {
        let bytes = if utf16 {
            let mut units = 0usize;
            loop {
                let pair = self.take(2)?;
                if pair == [0, 0] {
                    break;
                }
                units += 2;
            }
            self.buf[self.pos - units - 2..self.pos - 2].to_vec()
        } else {
            let mut len = 0usize;
            loop {
                if self.u8()? == 0 {
                    break;
                }
                len += 1;
            }
            self.buf[self.pos - len - 1..self.pos - 1].to_vec()
        };
        Ok(decode_text(&bytes, utf16, encoding))
    }
}

fn decode_text(bytes: &[u8], utf16: bool, encoding: &'static Encoding) -> String {
    let mut text = if utf16 {
        let units: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        String::from_utf16_lossy(&units)
    } else {
        encoding.decode(bytes).0.into_owned()
    };
    if text.starts_with('\u{feff}') {
        text.remove(0);
    }
    while text.ends_with('\0') {
        text.pop();
    }
    text
}

/// 解析后的 header 关键属性。
struct Header {
    v2: bool,
    utf16: bool,
    encoding: &'static Encoding,
    title: Option<String>,
    /// `Encrypted` 属性位标志：bit 1（值 2）= key 信息区固定密钥加密，
    /// 离线可解；bit 0（值 1）= 用户注册信息加密，无法离线解密。
    encrypt_bits: u8,
}

fn parse_header(buf: &[u8]) -> AppResult<(Header, usize)> {
    let mut reader = Reader::new(buf);
    let header_len = reader.u32_be()? as usize;
    if header_len == 0 || header_len + 8 > buf.len() {
        return Err(invalid("不是有效的 MDX 文件（header 长度异常）"));
    }
    let header_bytes = reader.take(header_len)?;
    let text = decode_text(header_bytes, true, UTF_16LE);

    // 属性是 `<Dictionary a="1" b="2" .../>` 形态；按引号切开后
    // 偶数段（从 1 数）是值、其前一段的尾部标识符是键。
    let mut attrs: Vec<(String, String)> = Vec::new();
    let parts: Vec<&str> = text.split('"').collect();
    let mut i = 1;
    while i < parts.len() {
        let value = parts[i];
        let key_part = parts[i - 1];
        // key_part 形如 `... Encrypted=`：跳过结尾的 `=` 再取尾部标识符。
        let key: String = key_part
            .chars()
            .rev()
            .skip_while(|c| !c.is_ascii_alphanumeric() && *c != '_')
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        if !key.is_empty() {
            attrs.push((key.to_ascii_lowercase(), value.to_string()));
        }
        i += 2;
    }
    let attr = |name: &str| attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone());

    let engine_version = attr("generatedbyengineversion")
        .or_else(|| attr("requiredformatversion"))
        .unwrap_or_default();
    let v2 = engine_version.trim().parse::<f64>().unwrap_or(0.0) >= 2.0;

    // 加密标志：No → 0；Yes → 1（用户注册信息加密）；数字按位解释。
    let encrypted = attr("encrypted").unwrap_or_default();
    let encrypted = encrypted.trim();
    let encrypt_bits = if encrypted.eq_ignore_ascii_case("no") || encrypted.is_empty() {
        0
    } else if encrypted.eq_ignore_ascii_case("yes") {
        1
    } else {
        encrypted.parse::<u8>().unwrap_or(0)
    };

    // MDD 资源包常缺 Encoding 属性（空值按 UTF-16 处理，与 mdict-js 一致）。
    let encoding_name = attr("encoding")
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "UTF-16".to_string());
    let encoding_name_normalized = encoding_name.trim().to_ascii_uppercase();
    let (encoding, utf16) = match encoding_name_normalized.as_str() {
        "UTF-8" | "UTF8" => (encoding_rs::UTF_8, false),
        "UTF-16" | "UTF16" | "UTF-16LE" => (UTF_16LE, true),
        "GBK" | "GB2312" | "GB18030" => (GBK, false),
        "BIG5" => (BIG5, false),
        other => {
            return Err(invalid(format!(
                "该词典使用不支持的文本编码 {other}（支持 UTF-8 / UTF-16 / GBK / Big5）"
            )));
        }
    };

    // header 之后固定跟 4 字节 adler32 校验（跳过，不校验）。
    Ok((
        Header {
            v2,
            utf16,
            encoding,
            title: attr("title"),
            encrypt_bits,
        },
        4 + header_len + 4,
    ))
}

/// MDict 容器索引：MDX（词头）与 MDD（资源路径）共用的解析结果。
struct ContainerIndex {
    header: Header,
    /// (key 文本, record 偏移)，保持文件内原始顺序。
    keys: Vec<(String, u64)>,
    record_blocks: Vec<RecordBlockLayout>,
    file: Vec<u8>,
}

/// 解析 MDict 容器（header / key 索引 / record 块布局）。
fn parse_container(file: Vec<u8>) -> AppResult<ContainerIndex> {
    let (header, key_section_start) = parse_header(&file)?;
    // bit 0（值 1）：绑定用户注册信息的加密，离线无法解密。
    if header.encrypt_bits & 0x01 != 0 {
        return Err(invalid(
            "该词典使用用户注册信息加密（Encrypted 含 1），无法离线解密导入",
        ));
    }
    let mut reader = Reader::new(&file[key_section_start..]);

    // ---- key 块区摘要 ----
    // v2.0 在词条数之后多一个「信息区解压长」，末尾多 4 字节校验和。
    let num_key_blocks = reader.number(header.v2)? as usize;
    let num_entries = reader.number(header.v2)? as usize;
    let key_index_decomp_len = if header.v2 {
        reader.number(header.v2)? as usize
    } else {
        0
    };
    let key_index_comp_len = reader.number(header.v2)? as usize;
    let key_blocks_len = reader.number(header.v2)? as usize;
    if header.v2 {
        reader.skip(4)?;
    }
    let key_index_raw = reader.take(key_index_comp_len)?;
    let key_blocks_raw = reader.take(key_blocks_len)?;

    // ---- key 块信息区：逐块(词条数 + 首词 + 尾词 + 压缩长 + 解压长) ----
    // bit 1（值 2）：信息区 [8..] 用 ripemd128 派生固定密钥加密（离线可解）。
    let mut key_index_decrypted;
    let key_index_raw = if header.v2 && header.encrypt_bits & 0x02 != 0 {
        key_index_decrypted = key_index_raw.to_vec();
        decrypt_key_block_info(&mut key_index_decrypted);
        &key_index_decrypted
    } else {
        key_index_raw
    };
    let key_index =
        decompress_block(key_index_raw, header.v2, key_index_decomp_len, "key 信息区")?;
    let mut info = Reader::new(&key_index);
    let mut blocks_meta = Vec::with_capacity(num_key_blocks.min(65536));
    for _ in 0..num_key_blocks {
        let _entries_in_block = info.number(header.v2)?;
        let _first = info.sized_text(header.v2, header.utf16, header.encoding)?;
        let _last = info.sized_text(header.v2, header.utf16, header.encoding)?;
        let comp_len = info.number(header.v2)? as usize;
        let decomp_len = info.number(header.v2)? as usize;
        blocks_meta.push((comp_len, decomp_len));
    }

    // ---- key 块：块内逐词条(记录偏移 + NUL 结尾词头) ----
    let mut keys: Vec<(String, u64)> = Vec::with_capacity(num_entries.min(1 << 22));
    let mut block_reader = Reader::new(key_blocks_raw);
    for &(comp_len, decomp_len) in &blocks_meta {
        let raw = block_reader.take(comp_len)?;
        let block = decompress_block(raw, header.v2, decomp_len, "key 块")?;
        let mut entry_reader = Reader::new(&block);
        while entry_reader.remaining() > 0 {
            let record_offset = entry_reader.number(header.v2)?;
            let key = entry_reader.nul_text(header.utf16, header.encoding)?;
            let key = key.trim();
            if key.is_empty() {
                continue;
            }
            keys.push((key.to_string(), record_offset));
        }
    }
    if keys.is_empty() {
        return Err(invalid("该 MDict 文件没有任何条目"));
    }
    if keys.len() != num_entries {
        tracing::warn!(
            expected = num_entries,
            actual = keys.len(),
            "mdict key count mismatch (continuing)"
        );
    }

    // ---- record 块区 ----
    let num_record_blocks = reader.number(header.v2)? as usize;
    let _num_record_entries = reader.number(header.v2)?;
    let record_index_len = reader.number(header.v2)? as usize;
    let record_blocks_len = reader.number(header.v2)? as usize;
    let record_index_raw = reader.take(record_index_len)?;
    let mut info = Reader::new(record_index_raw);
    let record_data_start = key_section_start + reader.pos;
    let mut record_blocks = Vec::with_capacity(num_record_blocks.min(65536));
    let mut file_offset = record_data_start;
    let mut stream_offset: u64 = 0;
    for _ in 0..num_record_blocks {
        let comp_len = info.number(header.v2)? as usize;
        let decomp_len = info.number(header.v2)? as usize;
        record_blocks.push(RecordBlockLayout {
            file_offset,
            comp_len,
            stream_offset,
            decomp_len,
        });
        file_offset += comp_len;
        stream_offset += decomp_len as u64;
    }
    if record_data_start + record_blocks_len > file.len() {
        // 个别引擎声明的 record 数据区比文件多几个字节，只告警不失败。
        tracing::warn!(
            declared = record_blocks_len,
            "mdict record data size exceeds file (continuing)"
        );
    }

    Ok(ContainerIndex {
        header,
        keys,
        record_blocks,
        file,
    })
}

/// 解包 MDD 资源包（与 .mdx 同容器，key 是资源路径）到 `out_dir`。
/// 返回写出的文件数；同偏移的别名 key 只写一次。
pub fn extract_mdd_resources(mdd_path: &Path, out_dir: &Path) -> AppResult<usize> {
    let file = std::fs::read(mdd_path)
        .map_err(|error| invalid(format!("无法读取 `{}`：{error}", mdd_path.display())))?;
    let container = parse_container(file)?;

    // 按偏移排序后逐 record 块顺序解包（同偏移别名去重）。
    let mut keys = container.keys;
    keys.sort_by(|a, b| a.1.cmp(&b.1));
    let mut unique: Vec<(String, u64)> = Vec::with_capacity(keys.len());
    for (path, offset) in keys {
        if unique.last().is_none_or(|&(_, prev)| prev != offset) {
            unique.push((path, offset));
        }
    }

    let file = &container.file;
    let mut written = 0usize;
    let mut index = 0usize;
    for block in &container.record_blocks {
        if index >= unique.len() {
            break;
        }
        let Some(raw) = file.get(block.file_offset..block.file_offset + block.comp_len) else {
            break;
        };
        let data = decompress_block(raw, container.header.v2, block.decomp_len, "record 块")?;
        let block_end = block.stream_offset + data.len() as u64;

        while index < unique.len() {
            let (path, offset) = &unique[index];
            if *offset < block.stream_offset {
                // 指向已扫过区域的悬空 key：跳过。
                index += 1;
                continue;
            }
            if *offset >= block_end {
                break;
            }
            let Some(relative) = crate::html_sanitize::normalize_resource_path(path) else {
                index += 1;
                continue;
            };
            let start = (offset - block.stream_offset) as usize;
            let end = unique
                .get(index + 1)
                .map(|&(_, next)| (next - block.stream_offset) as usize)
                .filter(|end| *end <= data.len())
                .unwrap_or(data.len());
            let destination = out_dir.join(&relative);
            if let Some(parent) = destination.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|error| invalid(format!("无法创建资源目录：{error}")))?;
            }
            std::fs::write(&destination, &data[start..end.max(start)])
                .map_err(|error| invalid(format!("无法写出资源 `{relative}`：{error}")))?;
            written += 1;
            index += 1;
        }
    }
    Ok(written)
}

impl MdxFile {
    /// 打开并解析 MDX 文件：读入全文件、解析 header 与 key 索引。
    /// record 块留给 [`MdxFile::entries`] 流式解压。
    pub fn open(path: &Path) -> AppResult<Self> {
        let file = std::fs::read(path)
            .map_err(|error| invalid(format!("无法读取 `{}`：{error}", path.display())))?;
        Self::parse(file)
    }

    fn parse(file: Vec<u8>) -> AppResult<Self> {
        let container = parse_container(file)?;
        let ContainerIndex {
            header,
            mut keys,
            record_blocks,
            file,
        } = container;

        // 同一 record 偏移的多个词头合并：首词头作 lemma，其余作词形变化。
        keys.sort_by(|a, b| a.1.cmp(&b.1));
        let mut groups: Vec<KeyGroup> = Vec::new();
        for (lemma, offset) in keys {
            match groups.last_mut() {
                Some(group) if group.record_offset == offset => {
                    group.inflections.push(lemma);
                }
                _ => groups.push(KeyGroup {
                    lemma,
                    inflections: Vec::new(),
                    record_offset: offset,
                }),
            }
        }

        Ok(Self {
            v2: header.v2,
            encoding: header.encoding,
            utf16: header.utf16,
            title: header.title,
            groups,
            record_blocks,
            file,
        })
    }

    /// 词典标题（header Title 属性），导入表单可用来预填名称。
    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    /// 词条数（词头别名合并后的组数），作为进度分母。
    pub fn entry_count(&self) -> usize {
        self.groups.len()
    }

    /// 流式产出词条：每次只在内存里保留当前 record 块的解压结果。
    pub fn entries(&self) -> MdxEntries<'_> {
        MdxEntries {
            file: self,
            group_index: 0,
            current: None,
        }
    }
}

/// 词条迭代器：按 record 偏移顺序单遍扫描，块按需解压、前进不复用。
pub struct MdxEntries<'a> {
    file: &'a MdxFile,
    group_index: usize,
    current: Option<(usize, Vec<u8>)>,
}

impl Iterator for MdxEntries<'_> {
    type Item = MdxEntry;

    fn next(&mut self) -> Option<MdxEntry> {
        let group = self.file.groups.get(self.group_index)?;
        self.group_index += 1;

        // 定位偏移所在的 record 块（组按偏移有序，块索引单调不减）。
        let blocks = &self.file.record_blocks;
        let in_range = |block: &RecordBlockLayout, inclusive: bool| {
            let end = block.stream_offset + block.decomp_len as u64;
            group.record_offset >= block.stream_offset
                && (group.record_offset < end || inclusive && group.record_offset == end)
        };
        let block_index = blocks
            .iter()
            .position(|block| in_range(block, false))
            .or_else(|| {
                // 个别引擎最后一个块的 decomp_len 声明偏小：允许命中末块边界。
                blocks
                    .last()
                    .is_some_and(|block| in_range(block, true) && block.decomp_len > 0)
                    .then_some(blocks.len() - 1)
            })?;

        if self.current.as_ref().map(|(index, _)| *index) != Some(block_index) {
            let block = &blocks[block_index];
            let raw = self
                .file
                .file
                .get(block.file_offset..block.file_offset + block.comp_len)?;
            let decompressed =
                match decompress_block(raw, self.file.v2, block.decomp_len, "record 块") {
                Ok(data) => data,
                Err(error) => {
                    tracing::warn!(block = block_index, %error, "mdx record block failed; stopping iteration");
                    return None;
                }
            };
            self.current = Some((block_index, decompressed));
        }
        let (_, block_data) = self.current.as_ref()?;
        let block = &blocks[block_index];
        let start = (group.record_offset - block.stream_offset) as usize;
        if start > block_data.len() {
            return None;
        }

        // 释义长度 = 到下一组偏移（同块内）或本块解压数据末尾。
        let next_offset = self.file.groups.get(self.group_index).map(|g| g.record_offset);
        let end = match next_offset {
            Some(next)
                if next >= block.stream_offset
                    && next < block.stream_offset + block.decomp_len as u64 =>
            {
                ((next - block.stream_offset) as usize).min(block_data.len())
            }
            _ => block_data.len(),
        };
        let bytes = block_data.get(start..end.max(start))?;

        Some(MdxEntry {
            lemma: group.lemma.clone(),
            inflections: group.inflections.clone(),
            text: decode_text(bytes, self.file.utf16, self.file.encoding),
        })
    }
}

// ---------- HTML → 纯文本 ----------

/// 块级标签：出现即换行（含 `<br>`）。
const BLOCK_TAGS: [&str; 26] = [
    "br", "p", "div", "li", "ul", "ol", "dl", "dt", "dd", "tr", "table", "tbody", "thead",
    "tfoot", "blockquote", "h1", "h2", "h3", "h4", "h5", "h6", "hr", "pre", "section",
    "article", "header",
];

/// 把 MDX 释义 HTML 转成带换行的纯文本：块级标签换行、`<td>/<th>`
/// 补空格分隔、`<style>/<script>` 整体丢弃、解码常用实体、丢弃空行。
pub fn html_to_plain_text(html: &str) -> String {
    let bytes = html.as_bytes();
    let mut out = String::with_capacity(html.len());
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'<' => {
                let Some(tag_end) = html[i..].find('>') else { break };
                let tag = &html[i + 1..i + tag_end];
                let name: String = tag
                    .trim_start_matches('/')
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric())
                    .collect::<String>()
                    .to_ascii_lowercase();
                match name.as_str() {
                    "script" | "style" => {
                        // 跳过到配对的闭合标签。
                        let rest = &html[i + tag_end + 1..];
                        let lower = rest.to_lowercase();
                        if let Some(close) = lower.find(&format!("</{name}")) {
                            i += tag_end + 1 + close;
                            match html[i..].find('>') {
                                Some(n) => i += n + 1,
                                None => break,
                            }
                        } else {
                            break;
                        }
                    }
                    "td" | "th" => {
                        // 单元格之间补一个空格（开/闭标签去重）。
                        if !out.ends_with([' ', '\n']) {
                            out.push(' ');
                        }
                        i += tag_end + 1;
                    }
                    _ => {
                        if BLOCK_TAGS.contains(&name.as_str()) {
                            out.push('\n');
                        }
                        i += tag_end + 1;
                    }
                }
            }
            b'&' => {
                // 实体：`&name;` / `&#123;` / `&#x1F;`，最长看 16 个字符。
                // 形似实体但查不到名字的（&ensp; 之类）整体丢弃；
                // 缺分号但完整匹配已知实体名的（老词典常见的 `&nbsp`）也解码；
                // 其余裸 `&`（如 AT&T）按字面保留。
                let rest = &html[i..];
                let mut entity_end = None;
                let mut run_end = None;
                for (offset, ch) in rest.char_indices().skip(1).take(16) {
                    if ch == ';' {
                        entity_end = Some(i + offset);
                        break;
                    }
                    if !(ch.is_ascii_alphanumeric() || ch == '#') {
                        run_end = Some(i + offset);
                        break;
                    }
                }
                match entity_end {
                    Some(end) => {
                        let entity = &html[i + 1..end];
                        if let Some(decoded) = decode_entity(entity) {
                            out.push_str(&decoded);
                        }
                        i = end + 1;
                    }
                    None => {
                        let run = &html[i + 1..run_end.unwrap_or(html.len().min(i + 17))];
                        match decode_entity(run) {
                            Some(decoded) if !run.is_empty() => {
                                out.push_str(&decoded);
                                i += 1 + run.len();
                            }
                            _ => {
                                out.push('&');
                                i += 1;
                            }
                        }
                    }
                }
            }
            _ => {
                // 普通文本（UTF-8 多字节不会是 '<' / '&'）整段拷贝。
                let run_end = bytes[i..]
                    .iter()
                    .position(|&b| b == b'<' || b == b'&')
                    .map(|n| i + n)
                    .unwrap_or(bytes.len());
                out.push_str(&html[i..run_end]);
                i = run_end;
            }
        }
    }

    // 收尾：剥掉老转换器的私有标记（`1`a`2` 字体记号、字面 `\n` 行分隔），
    // 去行尾空白、丢弃空行（弹窗场景取紧凑排版）、整体 trim。
    let mut marked = String::with_capacity(out.len());
    let mut chars = out.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '`' if chars.peek().is_some_and(|next| next.is_ascii_digit()) => {
                // `` `1`词头`2` `` 三字符字体记号：剥掉，边界补一个空格。
                chars.next();
                if chars.peek() == Some(&'`') {
                    chars.next();
                }
                marked.push(' ');
            }
            '\\' if chars.peek() == Some(&'n') => {
                marked.push('\n');
                chars.next();
            }
            _ => marked.push(c),
        }
    }
    let mut cleaned = String::with_capacity(marked.len());
    for line in marked.lines() {
        let line = line.trim_end();
        if !line.is_empty() {
            cleaned.push_str(line);
            cleaned.push('\n');
        }
    }
    cleaned.trim().to_string()
}

fn decode_entity(entity: &str) -> Option<String> {
    if let Some(numeric) = entity.strip_prefix('#') {
        let code = if let Some(hex) = numeric
            .strip_prefix('x')
            .or_else(|| numeric.strip_prefix('X'))
        {
            u32::from_str_radix(hex, 16).ok()?
        } else {
            numeric.parse::<u32>().ok()?
        };
        return char::from_u32(code).map(|c| c.to_string());
    }
    let decoded = match entity.to_ascii_lowercase().as_str() {
        "amp" => "&",
        "lt" => "<",
        "gt" => ">",
        "quot" => "\"",
        "apos" => "'",
        "nbsp" => "\u{a0}",
        "hellip" => "…",
        "mdash" => "—",
        "ndash" => "–",
        "lsquo" => "‘",
        "rsquo" => "’",
        "ldquo" => "“",
        "rdquo" => "”",
        "bull" => "•",
        "middot" => "·",
        "times" => "×",
        "divide" => "÷",
        "plusmn" => "±",
        "deg" => "°",
        "micro" => "µ",
        "copy" => "©",
        "reg" => "®",
        "trade" => "™",
        "sect" => "§",
        "para" => "¶",
        "dagger" => "†",
        "prime" => "′",
        "larr" => "←",
        "rarr" => "→",
        "uarr" => "↑",
        "darr" => "↓",
        "harr" => "↔",
        "minus" => "−",
        "infin" => "∞",
        "ne" => "≠",
        "le" => "≤",
        "ge" => "≥",
        "sup1" => "¹",
        "sup2" => "²",
        "sup3" => "³",
        "frac12" => "½",
        "frac14" => "¼",
        "frac34" => "¾",
        "sbquo" => "‚",
        "bdquo" => "„",
        _ => return None,
    };
    Some(decoded.to_string())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use flate2::write::ZlibEncoder;
    use flate2::Compression;
    use std::io::Write as _;

    fn zlib(data: &[u8]) -> Vec<u8> {
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(data).unwrap();
        encoder.finish().unwrap()
    }

    fn u32be(value: u64) -> Vec<u8> {
        (value as u32).to_be_bytes().to_vec()
    }

    /// MDict 压缩块：`[类型 4B = 2][adler32 占位 4B][zlib 数据]`。
    fn compressed_block(data: &[u8]) -> Vec<u8> {
        let mut out = vec![2, 0, 0, 0, 0, 0, 0, 0];
        out.extend_from_slice(&zlib(data));
        out
    }

    /// NUL 结尾文本（UTF-8/GBK 单字节形态）。
    fn utf8_nul_text(text: &str) -> Vec<u8> {
        let mut out = text.as_bytes().to_vec();
        out.push(0);
        out
    }

    /// 构造一个最小 v1.2 / UTF-8 MDX 字节流。
    /// 词条：hello@0、world@rec0、world2@rec0（别名）、zzz@rec0+rec1。
    fn minimal_mdx() -> Vec<u8> {
        let rec0 = "<div><b>hello</b> 你好<br/>second line</div>"
            .as_bytes()
            .to_vec();
        let rec1 = "<p>world &amp; <font color=\"red\">世界</font></p>"
            .as_bytes()
            .to_vec();
        let rec2 = "<p>zzz definition</p>".as_bytes().to_vec();

        // record 块（单块，压缩）。
        let mut stream = Vec::new();
        stream.extend_from_slice(&rec0);
        stream.extend_from_slice(&rec1);
        stream.extend_from_slice(&rec2);
        let record_block = compressed_block(&stream);
        let mut record_info = Vec::new();
        record_info.extend_from_slice(&u32be(record_block.len() as u64));
        record_info.extend_from_slice(&u32be(stream.len() as u64));

        // key 块（单块，压缩）：逐词条(记录偏移 u32 + NUL 结尾词头)。
        let offset_world = rec0.len() as u64;
        let offset_zzz = (rec0.len() + rec1.len()) as u64;
        let mut key_block_data = Vec::new();
        for (lemma, offset) in [
            ("hello", 0u64),
            ("world", offset_world),
            ("world2", offset_world),
            ("zzz", offset_zzz),
        ] {
            key_block_data.extend_from_slice(&u32be(offset));
            key_block_data.extend_from_slice(&utf8_nul_text(lemma));
        }
        let key_block = compressed_block(&key_block_data);

        // key 块信息区（未压缩，v1.2 未压缩块无类型前缀）：
        // 逐块(词条数 + 首词 + 尾词 + 压缩长 + 解压长)，词长前缀是 u8。
        let mut key_index = Vec::new();
        key_index.extend_from_slice(&u32be(4)); // 块内词条数
        key_index.push(5); // 首词长
        key_index.extend_from_slice(b"hello");
        key_index.push(3); // 尾词长
        key_index.extend_from_slice(b"zzz");
        key_index.extend_from_slice(&u32be(key_block.len() as u64));
        key_index.extend_from_slice(&u32be(key_block_data.len() as u64));

        let mut file = Vec::new();
        let header_text = "<Dictionary GeneratedByEngineVersion=\"1.2\" \
             RequiredFormatVersion=\"1.2\" Encrypted=\"No\" Encoding=\"UTF-8\" Title=\"Test\" />";
        let mut header_bytes = Vec::new();
        for unit in header_text.encode_utf16() {
            header_bytes.extend_from_slice(&unit.to_le_bytes());
        }
        file.extend_from_slice(&(header_bytes.len() as u32).to_be_bytes());
        file.extend_from_slice(&header_bytes);
        file.extend_from_slice(&[0, 0, 0, 0]); // header adler32（跳过）

        file.extend_from_slice(&u32be(1)); // num_key_blocks
        file.extend_from_slice(&u32be(4)); // num_entries
        file.extend_from_slice(&u32be(key_index.len() as u64)); // key 信息区压缩长
        file.extend_from_slice(&u32be(key_block.len() as u64)); // key 数据区长
        file.extend_from_slice(&key_index);
        file.extend_from_slice(&key_block);

        file.extend_from_slice(&u32be(1)); // num_record_blocks
        file.extend_from_slice(&u32be(4)); // num_record_entries
        file.extend_from_slice(&u32be(record_info.len() as u64));
        file.extend_from_slice(&u32be(record_block.len() as u64));
        file.extend_from_slice(&record_info);
        file.extend_from_slice(&record_block);
        file
    }

    #[test]
    fn extracts_mdd_resources_to_disk() {
        // v1.2 / UTF-8 容器，key 是资源路径：css/a.css + img/b.png。
        let css = b"body{color:red}".to_vec();
        let png = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        let mut stream = Vec::new();
        stream.extend_from_slice(&css);
        stream.extend_from_slice(&png);
        let record_block = compressed_block(&stream);
        let mut record_info = Vec::new();
        record_info.extend_from_slice(&u32be(record_block.len() as u64));
        record_info.extend_from_slice(&u32be(stream.len() as u64));

        let mut key_block_data = Vec::new();
        for (path, offset) in [
            ("\\css\\a.css", 0u64),
            ("/img/b.png", css.len() as u64),
        ] {
            key_block_data.extend_from_slice(&u32be(offset));
            key_block_data.extend_from_slice(&utf8_nul_text(path));
        }
        let key_block = compressed_block(&key_block_data);

        let mut key_index = Vec::new();
        key_index.extend_from_slice(&u32be(2)); // 块内条目数
        key_index.push(b"\\css\\a.css".len() as u8);
        key_index.extend_from_slice(b"\\css\\a.css");
        key_index.push(b"/img/b.png".len() as u8);
        key_index.extend_from_slice(b"/img/b.png");
        key_index.extend_from_slice(&u32be(key_block.len() as u64));
        key_index.extend_from_slice(&u32be(key_block_data.len() as u64));

        let mut file = Vec::new();
        let header_text = "<Library_Data GeneratedByEngineVersion=\"1.2\" \
             RequiredFormatVersion=\"1.2\" Encrypted=\"No\" Encoding=\"UTF-8\" />";
        let mut header_bytes = Vec::new();
        for unit in header_text.encode_utf16() {
            header_bytes.extend_from_slice(&unit.to_le_bytes());
        }
        file.extend_from_slice(&(header_bytes.len() as u32).to_be_bytes());
        file.extend_from_slice(&header_bytes);
        file.extend_from_slice(&[0, 0, 0, 0]);

        file.extend_from_slice(&u32be(1));
        file.extend_from_slice(&u32be(2));
        file.extend_from_slice(&u32be(key_index.len() as u64));
        file.extend_from_slice(&u32be(key_block.len() as u64));
        file.extend_from_slice(&key_index);
        file.extend_from_slice(&key_block);

        file.extend_from_slice(&u32be(1));
        file.extend_from_slice(&u32be(2));
        file.extend_from_slice(&u32be(record_info.len() as u64));
        file.extend_from_slice(&u32be(record_block.len() as u64));
        file.extend_from_slice(&record_info);
        file.extend_from_slice(&record_block);

        let dir = std::env::temp_dir().join("lingua-mdd-test");
        let _ = std::fs::remove_dir_all(&dir);
        let mdd_path = dir.join("test.mdd");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&mdd_path, file).unwrap();
        let out_dir = dir.join("resources");

        let written = extract_mdd_resources(&mdd_path, &out_dir).unwrap();
        assert_eq!(written, 2);
        assert_eq!(
            std::fs::read(out_dir.join("css/a.css")).unwrap(),
            b"body{color:red}".to_vec()
        );
        assert_eq!(std::fs::read(out_dir.join("img/b.png")).unwrap(), png);
    }

    #[test]
    fn parses_minimal_mdx_and_merges_alias_keys() {
        let dict = MdxFile::parse(minimal_mdx()).unwrap();
        assert_eq!(dict.title.as_deref(), Some("Test"));
        assert_eq!(dict.entry_count(), 3); // world2 合并进 world

        let entries: Vec<MdxEntry> = dict.entries().collect();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].lemma, "hello");
        assert!(entries[0].text.contains("second line"));
        assert_eq!(entries[1].lemma, "world");
        assert_eq!(entries[1].inflections, vec!["world2"]);
        assert!(entries[1].text.contains("&amp;"));
        assert_eq!(entries[2].lemma, "zzz");
    }

    #[test]
    fn entry_conversion_strips_html_and_decodes_entities() {
        let dict = MdxFile::parse(minimal_mdx()).unwrap();
        let entries: Vec<MdxEntry> = dict.entries().collect();
        let entry = entries[1].to_dictionary_entry(&Language::En, &Language::Zh, "test");
        assert_eq!(entry.lemma, "world");
        assert_eq!(entry.inflections, vec!["world2"]);
        assert_eq!(entry.definitions.len(), 1);
        assert_eq!(entry.definitions[0].definition, "world & 世界");
    }

    #[test]
    fn redirect_entry_is_detected() {
        let entry = MdxEntry {
            lemma: "abc".into(),
            inflections: vec![],
            text: "@@@LINK=ABC\r\n".into(),
        };
        assert!(entry.is_redirect());
    }

    #[test]
    fn user_id_encrypted_dictionary_is_rejected() {
        let mut bytes = minimal_mdx();
        // header 文本里把 Encrypted="No" 换成 Encrypted="1 "（等长，原地重写）。
        let header_len = u32::from_be_bytes(bytes[0..4].try_into().unwrap()) as usize;
        let header_text = decode_text(&bytes[4..4 + header_len], true, UTF_16LE);
        let patched = header_text.replace("Encrypted=\"No\"", "Encrypted=\"1 \"");
        assert_ne!(patched, header_text, "test fixture must contain Encrypted=\"No\"");
        let mut patched_bytes = Vec::new();
        for unit in patched.encode_utf16() {
            patched_bytes.extend_from_slice(&unit.to_le_bytes());
        }
        assert_eq!(patched_bytes.len(), header_len);
        bytes[4..4 + header_len].copy_from_slice(&patched_bytes);

        let error = match MdxFile::parse(bytes) {
            Err(error) => error,
            Ok(_) => panic!("user-id encrypted mdx must be rejected"),
        };
        assert!(error.to_string().contains("用户注册信息"), "{error}");
    }

    /// `_fast_decrypt` 的逆运算：密文半字节交换链正向加密，
    /// 供 fixture 构造 Encrypted=2 的 key 信息区。
    fn fast_encrypt(data: &mut [u8], key: &[u8; 16]) {
        let mut previous = 0x36u8;
        for (index, byte) in data.iter_mut().enumerate() {
            let plain = *byte;
            let cipher =
                (plain ^ previous ^ (index as u8) ^ key[index % key.len()]).rotate_left(4);
            *byte = cipher;
            previous = cipher;
        }
    }

    fn encrypt_key_block_info(block: &mut Vec<u8>) {
        let mut seed = [0u8; 8];
        seed[0..4].copy_from_slice(&block[4..8]);
        seed[4..8].copy_from_slice(&0x3695u32.to_le_bytes());
        let key: [u8; 16] = ripemd::Ripemd128::digest(seed).into();
        let mut tail = block.split_off(8);
        fast_encrypt(&mut tail, &key);
        block.extend(tail);
    }

    /// 构造最小 v2.0 / UTF-8 / Encrypted=2 MDX：key 信息区被固定密钥加密。
    fn minimal_mdx_v2_encrypted() -> Vec<u8> {
        let rec0 = "<div><b>hello</b> 你好</div>".as_bytes().to_vec();
        let rec1 = "<p>world &amp; 世界</p>".as_bytes().to_vec();

        let mut stream = Vec::new();
        stream.extend_from_slice(&rec0);
        stream.extend_from_slice(&rec1);
        let record_block = compressed_block(&stream);
        let mut record_info = Vec::new();
        record_info.extend_from_slice(&(record_block.len() as u64).to_be_bytes());
        record_info.extend_from_slice(&(stream.len() as u64).to_be_bytes());

        let offset_world = rec0.len() as u64;
        let mut key_block_data = Vec::new();
        for (lemma, offset) in [("hello", 0u64), ("world", offset_world)] {
            key_block_data.extend_from_slice(&offset.to_be_bytes());
            key_block_data.extend_from_slice(&utf8_nul_text(lemma));
        }
        let key_block = compressed_block(&key_block_data);

        // key 信息区：v2.0 词长前缀是 u16、文本后带 1 个 NUL 尾巴；
        // 整体 [类型 4B][adler32 占位 4B][zlib] 后再把 [8..] 加密。
        let mut key_index = Vec::new();
        key_index.extend_from_slice(&2u64.to_be_bytes()); // 块内词条数（8 字节槽）
        key_index.extend_from_slice(&5u16.to_be_bytes());
        key_index.extend_from_slice(b"hello");
        key_index.push(0); // v2.0 文本尾
        key_index.extend_from_slice(&5u16.to_be_bytes());
        key_index.extend_from_slice(b"world");
        key_index.push(0);
        key_index.extend_from_slice(&(key_block.len() as u64).to_be_bytes());
        key_index.extend_from_slice(&(key_block_data.len() as u64).to_be_bytes());

        let mut key_index_compressed = vec![2, 0, 0, 0, 0, 0, 0, 0];
        key_index_compressed.extend_from_slice(&zlib(&key_index));
        encrypt_key_block_info(&mut key_index_compressed);

        let mut file = Vec::new();
        let header_text = "<Dictionary GeneratedByEngineVersion=\"2.0\" \
             RequiredFormatVersion=\"2.0\" Encrypted=\"2\" Encoding=\"UTF-8\" Title=\"Enc\" />";
        let mut header_bytes = Vec::new();
        for unit in header_text.encode_utf16() {
            header_bytes.extend_from_slice(&unit.to_le_bytes());
        }
        file.extend_from_slice(&(header_bytes.len() as u32).to_be_bytes());
        file.extend_from_slice(&header_bytes);
        file.extend_from_slice(&[0, 0, 0, 0]);

        // v2.0 摘要：块数、词条数、信息区解压长、信息区压缩长、数据区长、4B 校验。
        file.extend_from_slice(&1u64.to_be_bytes());
        file.extend_from_slice(&2u64.to_be_bytes());
        file.extend_from_slice(&key_index.len().to_be_bytes());
        file.extend_from_slice(&key_index_compressed.len().to_be_bytes());
        file.extend_from_slice(&key_block.len().to_be_bytes());
        file.extend_from_slice(&[0, 0, 0, 0]);
        file.extend_from_slice(&key_index_compressed);
        file.extend_from_slice(&key_block);

        file.extend_from_slice(&1u64.to_be_bytes());
        file.extend_from_slice(&2u64.to_be_bytes());
        file.extend_from_slice(&record_info.len().to_be_bytes());
        file.extend_from_slice(&record_block.len().to_be_bytes());
        file.extend_from_slice(&record_info);
        file.extend_from_slice(&record_block);
        file
    }

    #[test]
    fn parses_v2_fixed_key_encrypted_mdx() {
        let dict = MdxFile::parse(minimal_mdx_v2_encrypted()).unwrap();
        assert_eq!(dict.title.as_deref(), Some("Enc"));
        assert_eq!(dict.entry_count(), 2);

        let entries: Vec<MdxEntry> = dict.entries().collect();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].lemma, "hello");
        assert_eq!(entries[1].lemma, "world");
        assert!(entries[1].text.contains("&amp;"));
    }

    #[test]
    fn end_to_end_import_and_lookup_via_sqlite() {
        // 解析 → DictionaryEntry → SQLite 入库 → 词头查询 的全链路。
        let dir = std::env::temp_dir().join("lingua-mdx-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.mdx");
        std::fs::write(&path, minimal_mdx()).unwrap();

        let dict = MdxFile::open(&path).unwrap();
        let database = lingua_database::Database::open_in_memory().unwrap();
        database.migrate().unwrap();
        let meta = lingua_database::DictionaryMeta {
            id: "test-mdx".into(),
            name: "Test MDX".into(),
            source_language: Language::En,
            gloss_language: Language::Zh,
        };
        let entries: Vec<DictionaryEntry> = dict
            .entries()
            .filter(|entry| !entry.is_redirect())
            .map(|entry| entry.to_dictionary_entry(&Language::En, &Language::Zh, "test-mdx"))
            .collect();
        assert_eq!(database.import_dictionary(&meta, &entries).unwrap(), 3);

        // 大小写不敏感命中，别名保留在词形变化里，富文本 HTML 随行。
        let hits = database.lookup_entries("WORLD", Some(Language::En)).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].lemma, "world");
        assert_eq!(hits[0].source, "test-mdx");
        assert_eq!(hits[0].inflections, vec!["world2"]);
        let html = hits[0].definitions[0].html.as_deref().unwrap();
        assert!(html.contains("<p>world &amp; <font"), "{html}");
    }

    #[test]
    fn html_to_plain_text_variants() {
        assert_eq!(html_to_plain_text("<p>a</p><p>b</p>"), "a\nb");
        assert_eq!(html_to_plain_text("a<br>b<br/>c"), "a\nb\nc");
        assert_eq!(html_to_plain_text("x&amp;y&hellip;z&#65;&#x42;"), "x&y…zAB");
        assert_eq!(
            html_to_plain_text("<style>.a{color:red}</style><b>keep</b>"),
            "keep"
        );
        assert_eq!(html_to_plain_text("<td>1</td><td>2</td>"), "1 2");
        assert_eq!(html_to_plain_text("\n\n\n  text  \n\n\n"), "text");
        // 未知实体整体丢弃；裸 & 保留；缺分号的已知实体也解码。
        assert_eq!(html_to_plain_text("a&unknownentity;b"), "ab");
        assert_eq!(html_to_plain_text("AT&T"), "AT&T");
        // 缺分号的已知实体也解码；行首 NBSP 缩进被整体 trim 掉。
        assert_eq!(html_to_plain_text("&nbsp&nbsp noun"), "noun");
        assert_eq!(html_to_plain_text("x&nbsp&nbsp noun"), "x\u{a0}\u{a0} noun");
        // 老转换器私有标记：`1`a`2` 字体记号、字面 \n 分行。
        assert_eq!(html_to_plain_text("`1`a`2`f.释义"), "a f.释义");
        assert_eq!(html_to_plain_text("灯伞座\\n灯罩架"), "灯伞座\n灯罩架");
    }
}
