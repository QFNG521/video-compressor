use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::thread::sleep;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

// ---------------- 数据结构 ----------------
#[derive(Clone, Serialize, Deserialize)]
struct Record {
    id: String,
    name: String,
    kind: String, // "video" | "image"
    status: String, // "done" | "error"
    src_size: u64,
    out_size: u64,
    out_name: String,
    encoder: String,
    quality: String,
    img_format: String,
    created: u64,
    // 输出不小于源 → 保留原文件（老记录无此字段，默认 false）
    #[serde(default)]
    skipped: bool,
}

#[derive(Clone, Serialize)]
struct ProgressPayload {
    id: String,
    name: String,
    status: String, // "running" | "done" | "error"
    progress: u8,
    speed: String,
    error: String,
}

#[derive(Clone, Serialize)]
struct CompressResult {
    id: String,
    name: String,
    kind: String,
    status: String,
    src_size: u64,
    out_size: u64,
    out_name: String,
    out_path: String,
    error: String,
    skipped: bool,
}

struct AppState {
    records: Mutex<Vec<Record>>,
    data_dir: PathBuf,
    output_dir: PathBuf,
    ffmpeg: PathBuf,
    gpu_encoders: Vec<String>,
}

// ---------------- 记录持久化 ----------------
fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn load_records(data_dir: &Path, output_dir: &Path) -> Vec<Record> {
    let file = data_dir.join("records.json");
    if let Ok(txt) = fs::read_to_string(&file) {
        if let Ok(v) = serde_json::from_str::<Vec<Record>>(&txt) {
            // 输出文件丢失的记录跳过，避免幽灵记录
            return v
                .into_iter()
                .filter(|r| r.status != "done" || output_dir.join(&r.out_name).exists())
                .collect();
        }
    }
    Vec::new()
}

fn save_records(state: &AppState) -> Result<(), String> {
    let recs = state.records.lock().unwrap();
    let json = serde_json::to_string_pretty(&*recs).map_err(|e| e.to_string())?;
    fs::write(state.data_dir.join("records.json"), json).map_err(|e| e.to_string())?;
    Ok(())
}

// ---------------- 命令 ----------------
#[tauri::command]
fn get_records(state: State<AppState>) -> Vec<Record> {
    state.records.lock().unwrap().clone()
}

#[tauri::command]
fn env_info(state: State<AppState>) -> serde_json::Value {
    let out = Command::new(&state.ffmpeg)
        .args(["-hide_banner", "-encoders"])
        .output();
    let txt = out
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();
    let webp = txt.contains("webp");
    serde_json::json!({
        "ffmpeg": true,
        "webp": webp,
        "output_dir": state.output_dir.to_string_lossy(),
        "gpu_encoders": state.gpu_encoders
    })
}

#[tauri::command]
fn delete_record(id: String, state: State<AppState>) -> Result<(), String> {
    let mut recs = state.records.lock().unwrap();
    if let Some(pos) = recs.iter().position(|r| r.id == id) {
        let out = state.output_dir.join(&recs[pos].out_name);
        let _ = fs::remove_file(&out);
        recs.remove(pos);
    }
    drop(recs);
    save_records(&state)
}

#[tauri::command]
fn clear_records(state: State<AppState>) -> Result<(), String> {
    let mut recs = state.records.lock().unwrap();
    for r in recs.iter() {
        let _ = fs::remove_file(state.output_dir.join(&r.out_name));
    }
    recs.clear();
    drop(recs);
    save_records(&state)
}

// ---------------- 进度解析 ----------------
fn parse_hms(s: &str) -> Option<f32> {
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() == 3 {
        let h: f32 = parts[0].parse().ok()?;
        let m: f32 = parts[1].parse().ok()?;
        let sec: f32 = parts[2].parse().ok()?;
        Some(h * 3600.0 + m * 60.0 + sec)
    } else {
        None
    }
}

/// 从 ffmpeg -progress 文件解析总时长、当前进度、速度
fn parse_progress(txt: &str) -> (Option<u8>, Option<String>) {
    let mut duration: Option<f32> = None;
    let mut out_time: Option<f32> = None;
    let mut speed: Option<String> = None;
    for line in txt.lines() {
        if let Some(v) = line.strip_prefix("duration=") {
            duration = parse_hms(v.trim());
        }
        if let Some(v) = line.strip_prefix("out_time=") {
            out_time = parse_hms(v.trim());
        }
        if let Some(v) = line.strip_prefix("speed=") {
            let s = v.trim();
            if s != "N/A" && !s.is_empty() {
                speed = Some(s.to_string());
            }
        }
    }
    let percent = match (duration, out_time) {
        (Some(d), Some(o)) if d > 0.0 => Some((o / d * 100.0).min(99.0) as u8),
        _ => None,
    };
    (percent, speed)
}

fn emit_progress(app: &AppHandle, id: &str, name: &str, status: &str, progress: u8, speed: &str) {
    let _ = app.emit(
        "progress",
        ProgressPayload {
            id: id.to_string(),
            name: name.to_string(),
            status: status.to_string(),
            progress,
            speed: speed.to_string(),
            error: String::new(),
        },
    );
}

// ---------------- 输出文件名去重 ----------------
fn unique_name(state: &AppState, stem: &str, suffix: &str) -> String {
    let recs = state.records.lock().unwrap();
    let taken: HashSet<String> = recs.iter().map(|r| r.out_name.clone()).collect();
    drop(recs);
    let mut out_name = format!("{}{}", stem, suffix);
    let mut i = 1;
    while taken.contains(&out_name) || state.output_dir.join(&out_name).exists() {
        i += 1;
        out_name = format!("{} ({}){}", stem, i, suffix);
    }
    out_name
}

/// CPU 视频编码参数（x265/x264）
fn cpu_video_args(encoder: &str, quality: &str) -> Result<(Vec<String>, String), String> {
    let crf: i32 = match (encoder, quality) {
        ("x265", "lossless") => 16,
        ("x265", "high") => 20,
        ("x265", "compact") => 24,
        (_, "lossless") => 15,
        (_, "high") => 18,
        (_, "compact") => 22,
        _ => 20,
    };
    let mut args: Vec<String> = vec!["-map".into(), "0:v:0".into(), "-map".into(), "0:a?".into()];
    if encoder == "x265" {
        args.extend([
            "-c:v".into(),
            "libx265".into(),
            "-preset".into(),
            "fast".into(),
            "-crf".into(),
            crf.to_string(),
            "-tag:v".into(),
            "hvc1".into(),
        ]);
    } else {
        args.extend([
            "-c:v".into(),
            "libx264".into(),
            "-preset".into(),
            "medium".into(),
            "-crf".into(),
            crf.to_string(),
        ]);
    }
    args.extend(["-pix_fmt".into(), "yuv420p".into()]);
    // 不探测音轨，统一重编码为 aac 160k（对所有源安全可用）
    args.extend(["-c:a".into(), "aac".into(), "-b:a".into(), "160k".into()]);
    args.extend(["-movflags".into(), "+faststart".into()]);
    let label = if encoder == "x265" { "x265".to_string() } else { "x264".to_string() };
    Ok((args, label))
}

/// GPU 视频编码参数：按本机可用编码器选择（优先 HEVC），返回 (args, 编码器短名)
fn gpu_video_args(state: &AppState) -> Option<(Vec<String>, String)> {
    let enc = state
        .gpu_encoders
        .iter()
        .find(|e| e.contains("hevc_"))
        .or_else(|| state.gpu_encoders.iter().find(|e| e.contains("h264_")))?;
    let params: Vec<String> = match enc.as_str() {
        // 数值按「更省体积」方向标定；输出仍比源大时由调用方回退保留原文件
        "hevc_nvenc" => vec!["-preset".into(), "p5".into(), "-cq".into(), "30".into()],
        "h264_nvenc" => vec!["-preset".into(), "p5".into(), "-cq".into(), "26".into()],
        "hevc_qsv" => vec!["-preset".into(), "medium".into(), "-global_quality".into(), "30".into()],
        "h264_qsv" => vec!["-preset".into(), "medium".into(), "-global_quality".into(), "26".into()],
        "hevc_amf" => vec![
            "-quality".into(), "quality".into(),
            "-rc".into(), "cqp".into(),
            "-qp_i".into(), "30".into(),
            "-qp_p".into(), "30".into(),
            "-qp_b".into(), "30".into(),
        ],
        "h264_amf" => vec![
            "-quality".into(), "quality".into(),
            "-rc".into(), "cqp".into(),
            "-qp_i".into(), "26".into(),
            "-qp_p".into(), "26".into(),
            "-qp_b".into(), "26".into(),
        ],
        // VideoToolbox：q:v 越大画质越好、文件越大（与常规 qscale 相反），故调低
        "hevc_videotoolbox" => vec!["-q:v".into(), "60".into()],
        _ => vec!["-q:v".into(), "50".into()], // h264_videotoolbox 及其余
    };
    let short = enc.split('_').nth(1).unwrap_or("gpu").to_string();
    let mut args: Vec<String> = vec![
        "-map".into(),
        "0:v:0".into(),
        "-map".into(),
        "0:a?".into(),
        "-c:v".into(),
        enc.clone(),
    ];
    args.extend(params);
    if enc.starts_with("hevc_") {
        args.extend(["-tag:v".into(), "hvc1".into()]);
    }
    args.extend(["-pix_fmt".into(), "yuv420p".into()]);
    args.extend(["-c:a".into(), "aac".into(), "-b:a".into(), "160k".into()]);
    args.extend(["-movflags".into(), "+faststart".into()]);
    Some((args, short))
}

// ---------------- 构造 ffmpeg 参数（复刻 Python 端逻辑） ----------------
fn build_args(
    state: &AppState,
    input: &Path,
    kind: &str,
    encoder: &str,
    quality: &str,
    img_format: &str,
) -> Result<(PathBuf, String, Vec<String>, String, String), String> {
    let stem = input
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "media".into());

    if kind == "image" {
        let ext = input
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        let valid = ["jpg", "jpeg", "png", "webp", "bmp", "tif", "tiff"];
        let want_webp = img_format == "webp";
        let mut suffix = if want_webp {
            ".webp".to_string()
        } else if valid.contains(&ext.as_str()) {
            format!(".{}", ext)
        } else {
            ".jpg".to_string()
        };
        // bmp/tif/tiff 强制转 webp 以获得更大压缩
        let force_webp = !want_webp && (suffix == ".bmp" || suffix == ".tif" || suffix == ".tiff");
        if force_webp {
            suffix = ".webp".to_string();
        }
        let out_name = unique_name(state, &stem, &suffix);
        let out_path = state.output_dir.join(&out_name);
        let q = match quality {
            "high" => 3,
            "balanced" => 6,
            _ => 9,
        };
        let mut args: Vec<String> = Vec::new();
        if suffix == ".png" {
            args.extend(["-compression_level".into(), "9".into()]);
        } else {
            args.extend(["-q:v".into(), q.to_string()]);
        }
        let final_format = if suffix == ".webp" {
            "webp".to_string()
        } else {
            "same".to_string()
        };
        Ok((out_path, out_name, args, final_format, String::new()))
    } else {
        // 视频
        let out_name = unique_name(state, &stem, ".mp4");
        let out_path = state.output_dir.join(&out_name);
        let (args, enc_label) = if encoder == "gpu" {
            match gpu_video_args(state) {
                Some((a, l)) => (a, l),
                // 无可用 GPU 编码器 → 回退 H.265（压缩率优先）
                None => cpu_video_args("x265", quality)?,
            }
        } else {
            cpu_video_args(encoder, quality)?
        };
        Ok((out_path, out_name, args, String::new(), enc_label))
    }
}

// ---------------- 压缩主命令 ----------------
#[tauri::command]
async fn compress(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    path: String,
    kind: String,
    encoder: String,
    quality: String,
    img_format: String,
) -> Result<CompressResult, String> {
    let input = PathBuf::from(&path);
    let src_size = fs::metadata(&input).map(|m| m.len()).unwrap_or(0);
    let name = input
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".into());

    let (out_path, out_name, args, final_format, resolved_enc) =
        build_args(&state, &input, &kind, &encoder, &quality, &img_format)?;

    emit_progress(&app, &id, &name, "running", 0, "");

    let prog_path = state.data_dir.join(format!(".prog_{}.txt", id));
    let _ = fs::write(&prog_path, "");

    let mut cmd = Command::new(&state.ffmpeg);
    // Windows：GUI 程序派生控制台子进程(ffmpeg.exe)默认会弹一个黑色控制台窗口，
    // 压缩结束自动关闭——用 CREATE_NO_WINDOW 隐藏它
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd.arg("-y").arg("-i").arg(&input);
    for a in &args {
        cmd.arg(a);
    }
    cmd.arg("-progress").arg(&prog_path).arg("-nostats");
    cmd.arg(&out_path);

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("无法启动 ffmpeg：{}", e))?;

    let mut last_speed = String::new();
    loop {
        if prog_path.exists() {
            if let Ok(txt) = fs::read_to_string(&prog_path) {
                let (pct, sp) = parse_progress(&txt);
                if let Some(s) = sp {
                    last_speed = s;
                }
                let p = pct.unwrap_or(0);
                emit_progress(&app, &id, &name, "running", p, &last_speed);
            }
        }
        match child.try_wait() {
            Ok(Some(_)) => {
                break;
            }
            Ok(None) => sleep(Duration::from_millis(400)),
            Err(e) => {
                emit_progress(&app, &id, &name, "error", 0, "");
                let _ = fs::remove_file(&prog_path);
                return Err(format!("ffmpeg 运行错误：{}", e));
            }
        }
    }

    let out_exists = out_path.exists() && fs::metadata(&out_path).map(|m| m.len() > 0).unwrap_or(false);

    let (status, mut out_size, error) = if out_exists {
        emit_progress(&app, &id, &name, "done", 100, &last_speed);
        (
            "done".to_string(),
            fs::metadata(&out_path).map(|m| m.len()).unwrap_or(0),
            String::new(),
        )
    } else {
        emit_progress(&app, &id, &name, "error", 0, "");
        (
            "error".to_string(),
            0,
            "编码失败（文件可能损坏或格式不支持）".to_string(),
        )
    };

    // 输出比源还大（小视频/已被高效压缩的源常见）→ 保留原文件作为结果，绝不越压越大
    let mut skipped = false;
    if status == "done" && src_size > 0 && out_size >= src_size {
        let _ = fs::copy(&input, &out_path);
        out_size = src_size;
        skipped = true;
    }

    let _ = fs::remove_file(&prog_path);

    {
        let mut recs = state.records.lock().unwrap();
        recs.push(Record {
            id: id.clone(),
            name: name.clone(),
            kind: kind.clone(),
            status: status.clone(),
            src_size,
            out_size,
            out_name: out_name.clone(),
            encoder: resolved_enc.clone(),
            quality: quality.clone(),
            img_format: final_format.clone(),
            created: now_secs(),
            skipped,
        });
    }
    save_records(&state)?;

    Ok(CompressResult {
        id,
        name,
        kind,
        status,
        src_size,
        out_size,
        out_name,
        out_path: out_path.to_string_lossy().to_string(),
        error,
        skipped,
    })
}

// ---------------- 启动 ----------------
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let res_dir = app.path().resource_dir().expect("resource_dir 不可用");
            // Windows 上 ffmpeg 可执行文件带 .exe 后缀
            let ffmpeg_name = if cfg!(target_os = "windows") { "ffmpeg.exe" } else { "ffmpeg" };
            let ffmpeg = res_dir.join("ffmpeg").join(ffmpeg_name);
            // 确保 ffmpeg 二进制有可执行权限（打包/拷贝后权限可能丢失；Windows 无此概念）
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = fs::set_permissions(&ffmpeg, fs::Permissions::from_mode(0o755));
            }
            let data_dir = app.path().app_data_dir().expect("app_data_dir 不可用");
            let output_dir = data_dir.join("compressed");
            fs::create_dir_all(&output_dir).ok();
            let records = load_records(&data_dir, &output_dir);
            // 探测本机可用的 GPU 硬件编码器（按优先级：NVIDIA > Intel QSV > AMD > Apple）
            let mut gpu_encoders = Vec::new();
            if let Ok(out) = Command::new(&ffmpeg).args(["-hide_banner", "-encoders"]).output() {
                let txt = String::from_utf8_lossy(&out.stdout);
                for name in [
                    "hevc_nvenc", "h264_nvenc", "hevc_qsv", "h264_qsv",
                    "hevc_amf", "h264_amf", "hevc_videotoolbox", "h264_videotoolbox",
                ] {
                    if txt.contains(name) {
                        gpu_encoders.push(name.to_string());
                    }
                }
            }
            app.manage(AppState {
                records: Mutex::new(records),
                data_dir,
                output_dir,
                ffmpeg,
                gpu_encoders,
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            compress,
            get_records,
            delete_record,
            clear_records,
            env_info
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
