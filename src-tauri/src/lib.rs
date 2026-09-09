mod commands;
mod images;
mod pdf;
mod watermark;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            commands::pick_files,
            commands::pick_folder,
            commands::pick_save_path,
            commands::preview_rename,
            commands::apply_rename,
            commands::open_in_folder,
            commands::open_url,
            commands::file_meta,
            images::image_meta,
            images::process_images,
            pdf::pdf_meta,
            pdf::pdf_merge,
            pdf::pdf_split,
            watermark::preview_watermark,
            watermark::process_photo_watermarks,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
