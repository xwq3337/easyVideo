pub mod crypto;
pub mod parse;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context};
use futures_util::StreamExt;
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::core::events::ProgressHandle;
use crypto::decrypt_aes128;
use parse::{KeyRef, MediaInfo, SegmentRef};

#[derive(Clone, Debug)]
pub struct ConcatPlan {
    pub list_file: PathBuf,
    pub segment_files: Vec<PathBuf>,
}

/// 转义 ffmpeg concat 列表里的路径。
///
/// ffmpeg 的 concat 解析器用单引号界定路径，路径里的 `'` 必须写成 `'\''`，否则会被当成
/// 引号结束。**实测**：`file '/tmp/My's Videos/seg.ts'` 被解析成 `/tmp/Mys` 并报
/// "Impossible to open" —— 用户只要把下载目录设在含撇号的路径下（如 `~/My's Videos`），
/// **每一次 HLS 下载都会在最后合并这一步失败**。
///
/// 实测有效的两种写法是 `'\''` 与全反斜杠转义；双引号包裹无效。这里取前者。
pub fn escape_concat_path(path: &str) -> String {
    path.replace('\'', "'\\''")
}

pub async fn download_media(
    client: &reqwest::Client,
    media: &MediaInfo,
    playlist_url: &str,
    task_dir: &Path,
    headers: &[(String, String)],
    concurrency: usize,
    progress: &ProgressHandle,
    cancel: CancellationToken,
) -> anyhow::Result<ConcatPlan> {
    use std::sync::Arc;
    std::fs::create_dir_all(task_dir).context("create task dir")?;
    // 分片与密钥的请求同样要带上浏览器扩展抓到的 Referer/UA ——
    // CDN 通常只校验其中某一类，漏掉就表现为"播放列表能下、分片全 403"。
    //
    // 但 Cookie/Authorization 属于**页面所在的域**。播放列表把分片或密钥指向
    // 另一个主机（CDN、对象存储、第三方密钥服务）时，原样带过去就是跨站泄露；
    // 浏览器不会这么做。跨主机只保留不敏感的上下文头。
    let same_map = Arc::new(crate::core::request::header_map(headers));
    let cross_map = Arc::new(crate::core::request::header_map(
        &crate::core::request::strip_credentials(headers),
    ));

    // HLS 进度按"已下分片数/总分片数"计数，GUI 用 ProgressUnit 决定展示格式
    progress.set_unit(crate::core::events::ProgressUnit::Segments);
    progress.set_total(media.segments.len() as u64);
    // ProgressHandle 挂在 TaskEntry 上，**跨单次运行存活**：失败后的 retry、暂停后的
    // unpause、重启后的 resume 都会再次进入本函数，而上一轮的计数仍留在 handle 里。
    // 本轮从零开始，否则重跑时那些"已在盘上、被续传跳过"的分片会再次计入，
    // 叠加上一次的运行结果，读数超过总数（用户看到 49 / 36 片）。
    progress.reset();

    let ext = if media.is_fmp4 { "m4s" } else { "ts" };
    let keys = Arc::new(Mutex::new(HashMap::new()));
    let sem = Arc::new(tokio::sync::Semaphore::new(concurrency.max(1)));
    let media_sequence = media.media_sequence;

    let mut handles = Vec::new();
    // (segments 下标, 媒体序号偏移):init 段不占媒体序号,见 plan_segment_indices 的说明
    for (idx, media_index) in plan_segment_indices(&media.segments) {
        let seg = &media.segments[idx];
        let file_path = task_dir.join(format!("seg-{idx:05}.{ext}"));
        // 先写临时名，成功后改名。"已完成"的唯一标志是**改名后的文件存在**。
        // 不能只用 exists() 判续传：写入路径是 create(截断) + write_all，
        // 若进程在两步之间被杀，会留下半截文件；下一轮会把它当成完成，
        // concat 列表照样列它，ffmpeg 产出损坏输出且**全程不报错**。
        // rename 在同一文件系统内是原子的，所以"存在"就等价于"完整"。
        let tmp_path = task_dir.join(format!("seg-{idx:05}.{ext}.part"));
        let client = client.clone();
        let seg_uri = seg.uri.clone();
        let key = seg.key.clone();
        let progress = progress.clone();
        let cancel = cancel.clone();
        let keys = keys.clone();
        // 按目标主机选头：同主机用全量，跨主机剥掉凭据。
        let seg_map = if crate::core::request::same_host(&seg_uri, playlist_url) {
            same_map.clone()
        } else {
            cross_map.clone()
        };
        let same_map_for_key = same_map.clone();
        let cross_map_for_key = cross_map.clone();
        let playlist_url = playlist_url.to_string();
        let permit = sem.clone().acquire_owned().await?;
        handles.push(tokio::spawn(async move {
            let _permit = permit;
            // 续传：只认已改名完成的文件；遗留的 .part 会被下面的 create 覆盖
            if file_path.exists() {
                progress.add(1);
                return Ok(file_path);
            }
            if cancel.is_cancelled() {
                return Err(anyhow!("canceled"));
            }
            let ciphertext = fetch_bytes(&client, &seg_uri, &seg_map, &cancel).await?;
            // 字节计数按**网络上传过来的密文长度**记，且必须在下面的 match 之前 ——
            // 无加密分支会把 ciphertext 整体 move 进 data。解密后的长度不等于下载量
            // （CBC 去填充会少几个字节），速度要反映的是真实吞吐。
            progress.add_bytes(ciphertext.len() as u64);
            let data = match &key {
                Some(KeyRef { uri, iv }) => {
                    let k = {
                        let mut cache = keys.lock().await;
                        if let Some(k) = cache.get(uri) {
                            *k
                        } else {
                            let key_map = if crate::core::request::same_host(uri, &playlist_url) {
                                &same_map_for_key
                            } else {
                                &cross_map_for_key
                            };
                            let raw = fetch_bytes(&client, uri, key_map, &cancel).await?;
                            let k: [u8; 16] = raw
                                .try_into()
                                .map_err(|_| anyhow!("key length != 16"))?;
                            cache.insert(uri.clone(), k);
                            k
                        }
                    };
                    // 用媒体序号而非数组下标推导 IV
                    let effective_iv = iv.unwrap_or_else(|| default_iv(media_sequence, media_index));
                    decrypt_aes128(&k, &effective_iv, &ciphertext)?
                }
                None => ciphertext,
            };
            let mut file = tokio::fs::File::create(&tmp_path).await?;
            file.write_all(&data).await?;
            file.flush().await?;
            drop(file); // 改名之前必须关闭句柄（Windows 上尤其如此）
            tokio::fs::rename(&tmp_path, &file_path).await?;
            progress.add(1);
            Ok::<_, anyhow::Error>(file_path)
        }));
    }
    // parse 阶段已把 init segment 排在首位,handle 顺序即 concat 顺序
    let mut files = Vec::new();
    for h in handles {
        files.push(h.await??);
    }

    // 写 ffmpeg concat 列表。列表的生成属于本模块（顺序是这里决定的），所以路径转义
    // 也放在这里，不要指望调用方再处理一遍。
    let list_file = task_dir.join("concat.txt");
    let mut list = String::new();
    for f in &files {
        list.push_str(&format!(
            "file '{}'\n",
            escape_concat_path(&f.to_string_lossy())
        ));
    }
    std::fs::write(&list_file, list)?;

    Ok(ConcatPlan {
        list_file,
        segment_files: files,
    })
}

async fn fetch_bytes(
    client: &reqwest::Client,
    uri: &str,
    headers: &reqwest::header::HeaderMap,
    cancel: &CancellationToken,
) -> anyhow::Result<Vec<u8>> {
    // 重试 3 次：指数退避 1s/2s/4s
    let mut last_err = None;
    for attempt in 0..3 {
        if cancel.is_cancelled() {
            anyhow::bail!("canceled");
        }
        match fetch_once(client, uri, headers).await {
            Ok(bytes) => return Ok(bytes),
            Err(e) => {
                last_err = Some(e);
                // 最后一次失败后不要再睡：那是纯粹的无效等待（每段白等 4 秒）。
                // 退避期间也必须能被取消打断，否则用户点了取消仍要等满退避。
                if attempt < 2 {
                    tokio::select! {
                        _ = cancel.cancelled() => anyhow::bail!("canceled"),
                        _ = tokio::time::sleep(std::time::Duration::from_secs(1 << attempt)) => {}
                    }
                }
            }
        }
    }
    Err(last_err.unwrap())
}

async fn fetch_once(
    client: &reqwest::Client,
    uri: &str,
    headers: &reqwest::header::HeaderMap,
) -> anyhow::Result<Vec<u8>> {
    let resp = client
        .get(uri)
        .headers(headers.clone())
        .send()
        .await?
        .error_for_status()?;
    let mut stream = resp.bytes_stream();
    let mut out = Vec::new();
    while let Some(chunk) = stream.next().await {
        out.extend_from_slice(&chunk?);
    }
    Ok(out)
}

/// 为一个 media playlist 规划每个分段的 `(segments 数组下标, 媒体序号偏移)`。
///
/// init 段(fMP4 的 `#EXT-X-MAP`)**不占媒体序号**,所以它之后的第一个媒体分片的媒体序号
/// 仍是 0。若直接拿数组下标当媒体序号去推导 IV,`#EXT-X-KEY` 未显式给 IV 时每个分片都会
/// 用错一个序号的 IV,解出乱码且不会报错。单独抽成纯函数就是为了能直接测这个不变式。
pub fn plan_segment_indices(segments: &[SegmentRef]) -> Vec<(usize, usize)> {
    let mut out = Vec::with_capacity(segments.len());
    let mut media_index = 0usize;
    for (idx, seg) in segments.iter().enumerate() {
        let mi = if seg.is_init {
            0 // init 段不使用 IV,占位
        } else {
            let v = media_index;
            media_index += 1;
            v
        };
        out.push((idx, mi));
    }
    out
}

fn default_iv(media_sequence: u64, segment_index: usize) -> [u8; 16] {
    let seq = media_sequence.wrapping_add(segment_index as u64);
    let mut iv = [0u8; 16];
    iv[8..].copy_from_slice(&seq.to_be_bytes());
    iv
}
