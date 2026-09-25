//! Ogg/Speex（.spx）按需解码为 WAV。
//!
//! MDict 时代的老日语发音词典（NHK 发音辞典等）音频大量使用
//! Speex，浏览器媒体管线不支持；`mdx-res://` 协议在服务 .spx 时
//! 就地转码成 16bit PCM WAV。文件只有几 KB、时长一两秒，
//! 每次点击解码是毫秒级，无需缓存或落盘。

use oxideav_speex::{SpeexHeader, SpeexStreamDecoder};

/// 从 Ogg 字节流抽取全部 packet。
/// Ogg 页格式固定（27 字节页头 + 分段表 + 载荷），packet 以
/// 值 <255 的分段结尾；文件来自本地资源目录，不做 CRC 校验。
fn ogg_packets(data: &[u8]) -> Option<Vec<Vec<u8>>> {
    let mut packets: Vec<Vec<u8>> = Vec::new();
    let mut current: Vec<u8> = Vec::new();
    let mut pos = 0usize;
    while pos + 27 <= data.len() {
        if &data[pos..pos + 4] != b"OggS" {
            return None;
        }
        let segment_count = data[pos + 26] as usize;
        let table_end = pos + 27 + segment_count;
        if table_end > data.len() {
            return None;
        }
        let mut payload = table_end;
        for &lacing in &data[pos + 27..table_end] {
            let lacing = lacing as usize;
            if payload + lacing > data.len() {
                return None;
            }
            current.extend_from_slice(&data[payload..payload + lacing]);
            payload += lacing;
            if lacing < 255 {
                packets.push(std::mem::take(&mut current));
            }
        }
        pos = payload;
    }
    Some(packets)
}

/// Ogg/Speex → 16bit 单声道 PCM WAV。解析失败（非 Speex / 损坏）
/// 返回 `None`，协议层按 404 处理。
pub fn speex_to_wav(data: &[u8]) -> Option<Vec<u8>> {
    let packets = ogg_packets(data)?;
    // Ogg/Speex 布局：packet 0 = Speex 头，packet 1 = 注释头，
    // 其后为音频帧。
    let header = SpeexHeader::parse(packets.first()?).ok()?;
    let mut decoder = SpeexStreamDecoder::for_header(&header).ok()?;
    let sample_rate = decoder.output_rate_hz();

    let mut pcm: Vec<i16> = Vec::new();
    for packet in packets.iter().skip(2) {
        match decoder.decode_packet_pcm_i16(packet) {
            Ok(samples) => pcm.extend_from_slice(&samples),
            Err(error) => {
                tracing::debug!(%error, "speex packet decode failed; skipping");
            }
        }
    }
    if pcm.is_empty() {
        return None;
    }
    Some(pcm_to_wav(&pcm, sample_rate))
}

fn pcm_to_wav(pcm: &[i16], sample_rate: u32) -> Vec<u8> {
    let data_len = pcm.len() * 2;
    let mut out = Vec::with_capacity(44 + data_len);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len as u32).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // fmt 块长度
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // 单声道
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&(sample_rate * 2).to_le_bytes()); // 字节率
    out.extend_from_slice(&2u16.to_le_bytes()); // 块对齐
    out.extend_from_slice(&16u16.to_le_bytes()); // 位深
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data_len as u32).to_le_bytes());
    for sample in pcm {
        out.extend_from_slice(&sample.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    /// 对真实 NHK 词典资源的解码验证（文件不在则跳过）：
    /// `cargo test -p lingua --lib speex -- --ignored --nocapture`
    #[test]
    #[ignore = "依赖用户机器上的 NHK 词典资源"]
    fn decodes_real_nhk_spx() {
        let path = std::path::Path::new(
            r"C:\Users\falwy\AppData\Roaming\com.taiyuuki.lingua\mdx\nhk-5c0f16\SPX\0000.spx",
        );
        let Ok(data) = std::fs::read(path) else {
            eprintln!("skip: fixture not found");
            return;
        };
        let wav = speex_to_wav(&data).expect("decode");
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        let sample_rate = u32::from_le_bytes(wav[24..28].try_into().unwrap());
        let data_len = u32::from_le_bytes(wav[40..44].try_into().unwrap()) as usize;
        eprintln!("wav: {} bytes, {sample_rate} Hz, {:.2}s", wav.len(), data_len as f64 / 2.0 / sample_rate as f64);
        assert!(sample_rate == 8000 || sample_rate == 16000 || sample_rate == 32000);
        assert!(data_len > 1600, "时长应明显大于 0.05s");
    }
}
