use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::Local;
use exr::image::write::write_rgb_file;
use image::{ImageBuffer, Rgb};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

#[derive(Default)]
struct WatcherState {
    watcher: Arc<Mutex<Option<RecommendedWatcher>>>,
    folder: Arc<Mutex<Option<PathBuf>>>,
    is_watching: Arc<Mutex<bool>>,
    recent_events: Arc<Mutex<HashMap<PathBuf, Instant>>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MergeRequest {
    paths: Vec<String>,
    output_dir: Option<String>,
    output_exr: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MergeResult {
    output_png_path: String,
    output_exr_path: Option<String>,
    width: u32,
    height: u32,
    merged_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ImageStat {
    path: String,
    average_luma: f32,
}

pub fn run() {
    tauri::Builder::default()
        .manage(WatcherState::default())
        .invoke_handler(tauri::generate_handler![
            watcher_set_folder,
            watcher_start,
            watcher_stop,
            watcher_is_running,
            analyze_images,
            merge_hdr,
        ])
        .run(tauri::generate_context!())
        .expect("error running tauri application");
}

#[tauri::command]
async fn watcher_set_folder(state: State<'_, WatcherState>, folder: String) -> Result<(), String> {
    let path = PathBuf::from(folder);
    if !path.exists() || !path.is_dir() {
        return Err("フォルダが存在しません".to_string());
    }

    let mut folder_state = state.folder.lock().map_err(|_| "lock error")?;
    *folder_state = Some(path);
    Ok(())
}

#[tauri::command]
async fn watcher_start(
    state: State<'_, WatcherState>,
    app_handle: AppHandle,
) -> Result<(), String> {
    let folder = {
        let folder_state = state.folder.lock().map_err(|_| "lock error")?;
        folder_state.clone().ok_or("監視フォルダが未設定です")?
    };

    let mut is_watching = state.is_watching.lock().map_err(|_| "lock error")?;
    if *is_watching {
        return Err("既に監視中です".to_string());
    }

    let recent_events = state.recent_events.clone();
    let app_handle_clone = app_handle.clone();

    let mut watcher = RecommendedWatcher::new(
        move |res: Result<Event, notify::Error>| {
            if let Ok(event) = res {
                let should_emit = matches!(
                    event.kind,
                    EventKind::Create(_) | EventKind::Modify(_)
                );

                if !should_emit {
                    return;
                }

                for path in event.paths {
                    if !should_process_file(&path) {
                        continue;
                    }

                    if !debounce_check(&path, &recent_events) {
                        continue;
                    }

                    let _ = app_handle_clone.emit(
                        "hdr://file-detected",
                        path.to_string_lossy().to_string(),
                    );
                }
            }
        },
        notify::Config::default(),
    )
    .map_err(|e| e.to_string())?;

    watcher
        .watch(&folder, RecursiveMode::Recursive)
        .map_err(|e| e.to_string())?;

    let mut watcher_state = state.watcher.lock().map_err(|_| "lock error")?;
    *watcher_state = Some(watcher);
    *is_watching = true;

    Ok(())
}

#[tauri::command]
async fn watcher_stop(state: State<'_, WatcherState>) -> Result<(), String> {
    let mut watcher_state = state.watcher.lock().map_err(|_| "lock error")?;
    let mut is_watching = state.is_watching.lock().map_err(|_| "lock error")?;
    *watcher_state = None;
    *is_watching = false;
    Ok(())
}

#[tauri::command]
async fn watcher_is_running(state: State<'_, WatcherState>) -> Result<bool, String> {
    let is_watching = state.is_watching.lock().map_err(|_| "lock error")?;
    Ok(*is_watching)
}

#[tauri::command]
async fn analyze_images(paths: Vec<String>) -> Result<Vec<ImageStat>, String> {
    if paths.is_empty() {
        return Err("解析対象がありません".to_string());
    }

    let mut stats = Vec::new();
    for path in paths {
        let image = load_rgb16(&path)?;
        let average_luma = calculate_average_luma(&image);
        stats.push(ImageStat { path, average_luma });
    }

    Ok(stats)
}

#[tauri::command]
async fn merge_hdr(request: MergeRequest) -> Result<MergeResult, String> {
    if request.paths.len() < 2 {
        return Err("合成には最低2枚必要です".to_string());
    }
    if request.paths.len() > 5 {
        return Err("合成は最大5枚までです".to_string());
    }

    let images: Vec<ImageBuffer<Rgb<u16>, Vec<u16>>> = request
        .paths
        .iter()
        .map(|path| load_rgb16(path))
        .collect::<Result<_, _>>()?;

    let width = images[0].width();
    let height = images[0].height();

    for image in &images {
        if image.width() != width || image.height() != height {
            return Err("画像サイズが一致しません".to_string());
        }
    }

    let mut merged = ImageBuffer::<Rgb<u16>, Vec<u16>>::new(width, height);

    for (x, y, pixel) in merged.enumerate_pixels_mut() {
        let mut sum_r: u64 = 0;
        let mut sum_g: u64 = 0;
        let mut sum_b: u64 = 0;

        for image in &images {
            let p = image.get_pixel(x, y);
            sum_r += p[0] as u64;
            sum_g += p[1] as u64;
            sum_b += p[2] as u64;
        }

        let count = images.len() as u64;
        let avg_r = (sum_r / count) as u16;
        let avg_g = (sum_g / count) as u16;
        let avg_b = (sum_b / count) as u16;

        *pixel = Rgb([avg_r, avg_g, avg_b]);
    }

    let output_dir = if let Some(dir) = request.output_dir {
        PathBuf::from(dir)
    } else {
        let first_path = PathBuf::from(&request.paths[0]);
        first_path
            .parent()
            .ok_or("出力先の決定に失敗しました")?
            .to_path_buf()
    };

    if !output_dir.exists() {
        std::fs::create_dir_all(&output_dir).map_err(|e| e.to_string())?;
    }

    let timestamp = Local::now().format("%Y%m%d_%H%M%S").to_string();
    let base_name = format!("hdr_merge_{}", timestamp);
    let png_path = output_dir.join(format!("{}.png", base_name));
    let exr_path = output_dir.join(format!("{}.exr", base_name));

    image::DynamicImage::ImageRgb16(merged.clone())
        .save(&png_path)
        .map_err(|e| e.to_string())?;

    let mut output_exr_path = None;
    if request.output_exr {
        let merged_ref = &merged;
        write_rgb_file(
            &exr_path,
            width as usize,
            height as usize,
            |x, y| {
                let pixel = merged_ref.get_pixel(x as u32, y as u32);
                (
                    pixel[0] as f32 / u16::MAX as f32,
                    pixel[1] as f32 / u16::MAX as f32,
                    pixel[2] as f32 / u16::MAX as f32,
                )
            },
        )
        .map_err(|e| e.to_string())?;

        output_exr_path = Some(exr_path.to_string_lossy().to_string());
    }

    Ok(MergeResult {
        output_png_path: png_path.to_string_lossy().to_string(),
        output_exr_path,
        width,
        height,
        merged_at: Local::now().to_rfc3339(),
    })
}

fn load_rgb16(path: &str) -> Result<ImageBuffer<Rgb<u16>, Vec<u16>>, String> {
    let image = image::open(path).map_err(|e| e.to_string())?;
    Ok(image.to_rgb16())
}

fn calculate_average_luma(image: &ImageBuffer<Rgb<u16>, Vec<u16>>) -> f32 {
    let mut total = 0.0f64;
    let pixel_count = (image.width() as f64) * (image.height() as f64);

    for pixel in image.pixels() {
        let r = pixel[0] as f64 / u16::MAX as f64;
        let g = pixel[1] as f64 / u16::MAX as f64;
        let b = pixel[2] as f64 / u16::MAX as f64;
        total += 0.2126 * r + 0.7152 * g + 0.0722 * b;
    }

    if pixel_count == 0.0 {
        return 0.0;
    }

    (total / pixel_count) as f32
}

fn should_process_file(path: &Path) -> bool {
    let ext = match path.extension().and_then(|ext| ext.to_str()) {
        Some(ext) => ext.to_lowercase(),
        None => return false,
    };

    matches!(ext.as_str(), "png" | "jpg" | "jpeg")
}

fn debounce_check(path: &Path, recent_events: &Arc<Mutex<HashMap<PathBuf, Instant>>>) -> bool {
    let mut map = match recent_events.lock() {
        Ok(guard) => guard,
        Err(_) => return false,
    };

    let now = Instant::now();
    if let Some(last) = map.get(path) {
        if now.duration_since(*last) < Duration::from_millis(500) {
            return false;
        }
    }

    map.insert(path.to_path_buf(), now);
    true
}
