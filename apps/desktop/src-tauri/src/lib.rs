//! 3MF Profile Transplant — Tauri 2 desktop shell over `wondermaker_3mf_core`.

mod commands;
mod config;
mod dto;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let result = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            commands::analyze_3mf,
            commands::validate_template,
            commands::convert_3mf,
            commands::path_exists,
            commands::open_output_folder,
            commands::get_config,
            commands::set_template_path,
            commands::suggest_output_path,
        ])
        .run(tauri::generate_context!());

    if let Err(e) = result {
        eprintln!("error while running tauri application: {e}");
        std::process::exit(1);
    }
}
