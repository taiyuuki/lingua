//! 发音库（AudioLib 形态的 .mdd）：按需随机读取 + 词头定位音频索引。
//!
//! 配对 .mdd（与 .mdx 同名）在词典导入时整体解包成文件，走
//! `mdx-res://` 文件服务；发音库是另一种形态——一个独立 .mdd 装着
//! **按词头命名**的音频（`word.spx` 美音 + `word_Uk.spx` 英音，或
//! `us\word.mp3` / `uk\word.mp3` 目录式），词典条目并不引用它们，
//! 点喇叭时按词头查。这类文件动辄几百 MB、二三十万条，整体解包
//! 等于复制出几十万个小文件；因此保留 .mdd 原件，只把 key 区解析
//! 进内存（几 MB），音频按需 seek + 解压单个 record 块读出。

use crate::html_sanitize::normalize_resource_path;
use crate::mdx::{RecordBlockLayout, Reader, decompress_block, parse_header, parse_key_section, parse_record_blocks};
use lingua_core::{AppError, AppResult};
use std::collections::HashMap;
use std::path::Path;

/// 按 key 随机读取的 .mdd：header / key 索引 / record 块布局常驻，
/// 音频数据留在磁盘上按需读取。线程安全（文件句柄在互斥锁内 seek）。
pub struct MddResourceFile {
    file: std::sync::Mutex<std::fs::File>,
    v2: bool,
    keys: Vec<String>,
    /// 规范化小写路径 → record 偏移（同偏移别名 key 共用一个偏移）。
    key_to_offset: HashMap<String, u64>,
    /// 升序去重后的全部 record 偏移（资源切片的右边界）。
    offsets: Vec<u64>,
    record_blocks: Vec<RecordBlockLayout>,
}

/// 从文件 `offset` 处精确读取 `len` 字节。
fn read_at(file: &mut std::fs::File, offset: usize, len: usize) -> AppResult<Vec<u8>> {
    use std::io::{Read as _, Seek as _, SeekFrom};
    let mut buffer = vec![0u8; len];
    file.seek(SeekFrom::Start(offset as u64))
        .map_err(|error| AppError::Internal(format!("mdd seek 失败：{error}")))?;
    file.read_exact(&mut buffer)
        .map_err(|error| AppError::Internal(format!("mdd 读取失败：{error}")))?;
    Ok(buffer)
}

impl MddResourceFile {
    /// 打开 .mdd 并解析全部元数据（header + key 区 + record 块索引）。
    /// 只读这几段（几 MB），不触碰 record 数据区。
    pub fn open(path: &Path) -> AppResult<Self> {
        let mut file = std::fs::File::open(path).map_err(|error| {
            AppError::InvalidInput(format!("无法读取 `{}`：{error}", path.display()))
        })?;
        let length_bytes = read_at(&mut file, 0, 4)?;
        let header_len = u32::from_be_bytes([length_bytes[0], length_bytes[1], length_bytes[2], length_bytes[3]])
            as usize;
        // parse_header 只从头读（长度 + 属性文本 + 校验），传前缀即可。
        let prefix = read_at(&mut file, 0, 4 + header_len + 4)?;
        let (header, key_section_start) = parse_header(&prefix)?;
        if header.encrypt_bits & 0x01 != 0 {
            return Err(AppError::InvalidInput(
                "该发音库使用用户注册信息加密（Encrypted 含 1），无法离线解密导入".to_string(),
            ));
        }
        let v2 = header.v2;

        // key 区摘要：v2.0 = 5 个 8 字节数字 + 4 字节校验；v1.2 = 4 个 4 字节数字。
        let key_summary_len = if v2 { 44 } else { 16 };
        let summary = read_at(&mut file, key_section_start, key_summary_len)?;
        let mut reader = Reader::new(&summary);
        let num_key_blocks = reader.number(v2)? as usize;
        let num_entries = reader.number(v2)? as usize;
        let key_index_decomp_len = if v2 { reader.number(v2)? as usize } else { 0 };
        let key_index_comp_len = reader.number(v2)? as usize;
        let key_blocks_len = reader.number(v2)? as usize;
        let key_body_start = key_section_start + key_summary_len;
        let key_index_raw = read_at(&mut file, key_body_start, key_index_comp_len)?;
        let key_blocks_raw = read_at(&mut file, key_body_start + key_index_comp_len, key_blocks_len)?;
        let keys = parse_key_section(
            &header,
            num_key_blocks,
            num_entries,
            key_index_decomp_len,
            &key_index_raw,
            &key_blocks_raw,
        )?;

        // record 区摘要（4 个数字）+ 索引。
        let record_summary_len = if v2 { 32 } else { 16 };
        let record_summary_at = key_body_start + key_index_comp_len + key_blocks_len;
        let record_summary = read_at(&mut file, record_summary_at, record_summary_len)?;
        let mut record_reader = Reader::new(&record_summary);
        let num_record_blocks = record_reader.number(v2)? as usize;
        let _num_record_entries = record_reader.number(v2)? as usize;
        let record_index_len = record_reader.number(v2)? as usize;
        let record_index_at = record_summary_at + record_summary_len;
        let record_index_raw = read_at(&mut file, record_index_at, record_index_len)?;
        let record_data_start = record_index_at + record_index_len;
        let record_blocks =
            parse_record_blocks(v2, &record_index_raw, record_data_start, num_record_blocks)?;

        let mut key_to_offset = HashMap::with_capacity(keys.len());
        for (key, offset) in &keys {
            if let Some(normalized) = normalize_resource_path(key) {
                key_to_offset
                    .entry(normalized.to_lowercase())
                    .or_insert(*offset);
            }
        }
        let mut sorted: Vec<u64> = keys.iter().map(|(_, offset)| *offset).collect();
        sorted.sort_unstable();
        sorted.dedup();

        Ok(Self {
            file: std::sync::Mutex::new(file),
            v2,
            keys: keys.into_iter().map(|(key, _)| key).collect(),
            key_to_offset,
            offsets: sorted,
            record_blocks,
        })
    }

    /// 全部 key（文件内原始路径，导入建索引用）。
    pub fn keys(&self) -> &[String] {
        &self.keys
    }

    /// 按规范化路径读取一个资源的原始字节；不存在返回 `None`。
    /// 路径大小写不敏感（`\UK\Word.MP3` 与 `uk/word.mp3` 等价）。
    pub fn read_resource(&self, path: &str) -> AppResult<Option<Vec<u8>>> {
        let Some(normalized) = normalize_resource_path(path) else {
            return Ok(None);
        };
        let Some(&offset) = self.key_to_offset.get(&normalized.to_lowercase()) else {
            return Ok(None);
        };
        // 定位所在 record 块（块按 stream_offset 升序）。
        let block_index = self
            .record_blocks
            .partition_point(|block| block.stream_offset <= offset);
        if block_index == 0 {
            return Ok(None);
        }
        let block = &self.record_blocks[block_index - 1];
        if offset >= block.stream_offset + block.decomp_len as u64 {
            // 偏移落在块缝隙（引擎数据异常），按缺失处理。
            return Ok(None);
        }
        let raw = {
            let mut file = self.file.lock().map_err(|_| {
                AppError::Internal("mdd 文件句柄锁中毒".to_string())
            })?;
            read_at(&mut file, block.file_offset, block.comp_len)?
        };
        let data = decompress_block(&raw, self.v2, block.decomp_len, "record 块")?;
        let start = (offset - block.stream_offset) as usize;
        if start > data.len() {
            return Ok(None);
        }
        // 资源长度 = 到下一个 key 偏移（跨块时截到本块末尾）。
        let end = self
            .offsets
            .get(self.offsets.partition_point(|&next| next <= offset))
            .map(|next| ((next - block.stream_offset) as usize).min(data.len()))
            .unwrap_or(data.len());
        Ok(Some(data[start..end.max(start)].to_vec()))
    }
}

// ---------- 词头音频索引提取 ----------

/// 口音。绝大多数英英/英汉词库只有英/美两种标注。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Accent {
    Uk,
    Us,
}

impl Accent {
    pub fn as_str(self) -> &'static str {
        match self {
            Accent::Uk => "uk",
            Accent::Us => "us",
        }
    }
}

/// 一条词头音频索引：`(词头小写, 口音)` → mdd 内规范化资源路径。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioEntry {
    pub word: String,
    pub accent: Accent,
    pub resource_path: String,
}

/// 音频扩展名（与前端 AUDIO_URL 判定、mdx-res Content-Type 对齐）。
const AUDIO_EXTENSIONS: [&str; 10] = [
    "mp3", "wav", "ogg", "oga", "m4a", "aac", "spx", "opus", "flac", "webm",
];

/// 口音目录 / 后缀记号 → 口音（uk|gb → 英，us → 美）。
fn accent_from_token(token: &str) -> Option<Accent> {
    match token.to_ascii_lowercase().as_str() {
        "uk" | "gb" => Some(Accent::Uk),
        "us" => Some(Accent::Us),
        _ => None,
    }
}

/// 大小写不敏感地去掉 `text` 结尾的任一后缀，返回剩余部分。
fn strip_suffix_ci<'a>(text: &'a str, suffixes: &[&str]) -> Option<&'a str> {
    suffixes.iter().find_map(|suffix| {
        (text.len() > suffix.len()
            && text[(text.len() - suffix.len())..].eq_ignore_ascii_case(suffix))
            .then(|| &text[..text.len() - suffix.len()])
    })
}

/// 从 .mdd 的 key 清单提取「词头 → 音频」索引。
///
/// 支持现实中的三类命名（大小写不敏感）：
///
/// * 裸名 + 口音后缀：`word.spx`（默认美音）+ `word_Uk.spx`（英音），
///   即流传最广的英式美式发音库；
/// * 口音目录：`us\word.mp3` / `uk\word__gb_1.spx`（牛津 8 的配对 mdd
///   也能当独立发音库用）；
/// * 口音 + 同词序号后缀：`_speak_1_us_3.spx` → 词头 `speak`、美音。
///
/// 纯数字名（NHK 的 `SPX\0001.spx`，音频只能由其 mdx 正文引用）与
/// 非音频文件一律跳过。
pub fn extract_audio_entries(keys: &[String]) -> Vec<AudioEntry> {
    let mut seen: std::collections::HashSet<(String, Accent)> = std::collections::HashSet::new();
    let mut entries = Vec::new();
    for key in keys {
        let Some(entry) = audio_entry_for_key(key) else {
            continue;
        };
        if seen.insert((entry.word.clone(), entry.accent)) {
            entries.push(entry);
        }
    }
    entries
}

fn audio_entry_for_key(key: &str) -> Option<AudioEntry> {
    let path = normalize_resource_path(key)?;
    // 倒数第二段作为口音目录提示（us/、uk/…）；更深的父目录不参与。
    let mut segments = path.split('/');
    let file_name = segments.next_back().unwrap_or_default();
    let dir = segments.next_back();
    let mut accent = dir.and_then(accent_from_token);

    let (base, extension) = file_name.rsplit_once('.')?;
    if !AUDIO_EXTENSIONS.contains(&extension.to_ascii_lowercase().as_str()) {
        return None;
    }

    // 反复剥结尾的同词序号（`_<1-3 位数字>`）与口音后缀，中间吞掉
    // `__` 双下划线残留：`word__gb_1` → `word`，`_speak_1_us_3` →
    // `speak`，`word_Uk` → `word`。未识别出口音的裸名按美音
    // （英式美式发音库的通行约定）。
    let mut rest = base;
    loop {
        let trimmed = rest.trim_end_matches('_');
        if trimmed.len() >= 2 && trimmed.len() < rest.len() {
            rest = trimmed;
            continue;
        }
        if let Some(head) = strip_suffix_ci(rest, &["_uk", "_gb", "_us"]) {
            // 目录是更强的口音线索；没有目录提示时才采纳后缀记号
            // （`status` 这类词尾不会被当作 `_us` 剥掉——见下方单测）。
            if accent.is_none() {
                accent = accent_from_token(&rest[head.len() + 1..]);
            }
            rest = head;
            continue;
        }
        let no_digits = rest.trim_end_matches(|c: char| c.is_ascii_digit());
        if rest.len() - no_digits.len() >= 1
            && rest.len() - no_digits.len() <= 3
            && no_digits.len() >= 2
            && no_digits.ends_with('_')
        {
            rest = no_digits;
            continue;
        }
        break;
    }
    let word = rest.trim_matches('_');
    if word.is_empty() || !word.chars().any(char::is_alphabetic) {
        return None;
    }
    Some(AudioEntry {
        word: word.to_lowercase(),
        accent: accent.unwrap_or(Accent::Us),
        resource_path: path,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use flate2::write::ZlibEncoder;
    use flate2::Compression;
    use std::io::Write as _;

    #[test]
    fn extracts_word_audio_from_realistic_key_styles() {
        let keys: Vec<String> = [
            r"\'bout.spx",                 // 裸名（美音）
            r"\'bout_Uk.spx",              // 英音后缀
            r"\uk\accidence__gb_1.spx",    // 口音目录 + gb 后缀 + 序号
            r"\us\_speak_1_us_3.spx",      // 前缀词 + 序号 + 口音 + 序号
            r"\pic\accordion.jpg",         // 非音频
            r"\SPX\0001.spx",              // 纯数字（NHK）
            r"\us_pron.png",               // 非音频
            r"\hello.MP3",                 // 大写扩展名
        ]
        .into_iter()
        .map(str::to_string)
        .collect();
        let entries = extract_audio_entries(&keys);
        let find = |word: &str, accent: Accent| {
            entries
                .iter()
                .find(|entry| entry.word == word && entry.accent == accent)
        };
        assert_eq!(find("'bout", Accent::Us).unwrap().resource_path, "'bout.spx");
        assert_eq!(find("'bout", Accent::Uk).unwrap().resource_path, "'bout_Uk.spx");
        let gb = find("accidence", Accent::Uk).unwrap();
        assert_eq!(gb.resource_path, "uk/accidence__gb_1.spx");
        assert_eq!(find("speak", Accent::Us).unwrap().resource_path, "us/_speak_1_us_3.spx");
        assert_eq!(find("hello", Accent::Us).unwrap().resource_path, "hello.MP3");
        // 图片 / 纯数字一律不进索引。
        assert_eq!(entries.len(), 5, "{entries:?}");
    }

    #[test]
    fn suffix_colliding_words_keep_their_word() {
        // 词尾恰好是 bus/status 这类含 us 的词：`_us` 只剥独立的 `_us` 记号。
        let keys: Vec<String> = [r"\status.spx", r"\bus.spx"]
            .into_iter()
            .map(str::to_string)
            .collect();
        let entries = extract_audio_entries(&keys);
        let words: Vec<&str> = entries.iter().map(|entry| entry.word.as_str()).collect();
        assert!(words.contains(&"status"), "{words:?}");
        assert!(words.contains(&"bus"), "{words:?}");
    }

    // ---------- MddResourceFile 随机读取（构造最小 v2.0 .mdd） ----------

    fn zlib_bytes(data: &[u8]) -> Vec<u8> {
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(data).unwrap();
        encoder.finish().unwrap()
    }

    /// MDict v2.0 压缩块：`[类型 4B = 2][adler32 占位 4B][zlib 数据]`。
    fn compressed_block(data: &[u8]) -> Vec<u8> {
        let mut out = vec![2, 0, 0, 0, 0, 0, 0, 0];
        out.extend_from_slice(&zlib_bytes(data));
        out
    }

    /// 构造一个 v2.0 / UTF-8 的 .mdd：us/hello.mp3 + uk/hello_Uk.spx +
    /// pic/a.jpg，单 record 块。
    fn build_mdd() -> Vec<u8> {
        let mp3 = b"ID3hello-mp3".to_vec();
        let spx = vec![0x89, b'S', b'P', b'X', 1, 2, 3, 4, 5, 6];
        let jpg = b"\xFF\xD8\xFFjpg".to_vec();

        let mut stream = Vec::new();
        stream.extend_from_slice(&mp3);
        stream.extend_from_slice(&spx);
        stream.extend_from_slice(&jpg);
        let record_block = compressed_block(&stream);
        let mut record_info = Vec::new();
        record_info.extend_from_slice(&(record_block.len() as u64).to_be_bytes());
        record_info.extend_from_slice(&(stream.len() as u64).to_be_bytes());

        let mut key_block_data = Vec::new();
        for (key, offset) in [
            (r"\us\hello.mp3", 0u64),
            (r"\uk\hello_Uk.spx", mp3.len() as u64),
            (r"\pic\a.jpg", (mp3.len() + spx.len()) as u64),
        ] {
            key_block_data.extend_from_slice(&offset.to_be_bytes());
            key_block_data.extend_from_slice(key.as_bytes());
            key_block_data.push(0);
        }
        let key_block = compressed_block(&key_block_data);

        // key 信息区（v2.0：词条数 + 首词 + 尾词(长度前缀 u16 + 文本 + 尾字节) + 压缩/解压长）。
        // 首尾词按字典序：`\pic\a.jpg` < `\uk\hello_Uk.spx` < `\us\hello.mp3`。
        let mut key_index = Vec::new();
        key_index.extend_from_slice(&3u64.to_be_bytes());
        for text in [r"\pic\a.jpg", r"\us\hello.mp3"] {
            key_index.extend_from_slice(&(text.len() as u16).to_be_bytes());
            key_index.extend_from_slice(text.as_bytes());
            key_index.push(0);
        }
        key_index.extend_from_slice(&(key_block.len() as u64).to_be_bytes());
        key_index.extend_from_slice(&(key_block_data.len() as u64).to_be_bytes());
        let key_index_compressed = compressed_block(&key_index);

        let mut file = Vec::new();
        let header_text = "<Library_Data GeneratedByEngineVersion=\"2.0\" \
             RequiredFormatVersion=\"2.0\" Encrypted=\"No\" Encoding=\"UTF-8\" />";
        let mut header_bytes = Vec::new();
        for unit in header_text.encode_utf16() {
            header_bytes.extend_from_slice(&unit.to_le_bytes());
        }
        file.extend_from_slice(&(header_bytes.len() as u32).to_be_bytes());
        file.extend_from_slice(&header_bytes);
        file.extend_from_slice(&[0, 0, 0, 0]); // header adler32（跳过）

        // v2.0 摘要：块数、词条数、信息区解压长、信息区压缩长、数据区长、4B 校验。
        file.extend_from_slice(&1u64.to_be_bytes());
        file.extend_from_slice(&3u64.to_be_bytes());
        file.extend_from_slice(&(key_index.len() as u64).to_be_bytes());
        file.extend_from_slice(&(key_index_compressed.len() as u64).to_be_bytes());
        file.extend_from_slice(&(key_block.len() as u64).to_be_bytes());
        file.extend_from_slice(&[0, 0, 0, 0]);
        file.extend_from_slice(&key_index_compressed);
        file.extend_from_slice(&key_block);

        file.extend_from_slice(&1u64.to_be_bytes());
        file.extend_from_slice(&3u64.to_be_bytes());
        file.extend_from_slice(&(record_info.len() as u64).to_be_bytes());
        file.extend_from_slice(&(record_block.len() as u64).to_be_bytes());
        file.extend_from_slice(&record_info);
        file.extend_from_slice(&record_block);
        file
    }

    #[test]
    fn random_reads_return_exact_resource_bytes() {
        let dir = std::env::temp_dir().join("lingua-audio-lib-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mdd_path = dir.join("lib.mdd");
        std::fs::write(&mdd_path, build_mdd()).unwrap();

        let library = MddResourceFile::open(&mdd_path).unwrap();
        assert_eq!(library.keys().len(), 3);

        assert_eq!(
            library.read_resource("us/hello.mp3").unwrap().unwrap(),
            b"ID3hello-mp3".to_vec()
        );
        // 大小写 / 分隔符不敏感。
        assert_eq!(
            library.read_resource(r"\US\HELLO.MP3").unwrap().unwrap(),
            b"ID3hello-mp3".to_vec()
        );
        assert_eq!(
            library.read_resource("uk/hello_Uk.spx").unwrap().unwrap(),
            vec![0x89, b'S', b'P', b'X', 1, 2, 3, 4, 5, 6]
        );
        assert_eq!(
            library.read_resource("pic/a.jpg").unwrap().unwrap(),
            b"\xFF\xD8\xFFjpg".to_vec()
        );
        assert!(library.read_resource("pic/missing.png").unwrap().is_none());
        assert!(library.read_resource("../escape").unwrap().is_none());
    }

    /// 真机发音库全链路（默认忽略，本机有文件时 `--ignored` 显式跑）。
    #[test]
    #[ignore = "需要本机的英式美式发音库文件"]
    fn opens_real_sound_library() {
        let path = Path::new(
            r"C:\Users\falwy\Documents\dict词库\发音库\英式美式发音库\Sound_us_uk.mdd",
        );
        if !path.is_file() {
            return;
        }
        let library = MddResourceFile::open(path).unwrap();
        assert!(library.keys().len() > 200_000, "{}", library.keys().len());

        let entries = extract_audio_entries(library.keys());
        assert!(entries.len() > 200_000, "{}", entries.len());
        // the 的英/美音都应在。
        assert!(entries
            .iter()
            .any(|entry| entry.word == "the" && entry.accent == Accent::Us));
        assert!(entries
            .iter()
            .any(|entry| entry.word == "the" && entry.accent == Accent::Uk));

        let the = entries
            .iter()
            .find(|entry| entry.word == "the" && entry.accent == Accent::Us)
            .unwrap();
        let data = library.read_resource(&the.resource_path).unwrap().unwrap();
        assert!(data.len() > 500, "{}", data.len());
    }
}
