#![windows_subsystem = "windows"]
mod annotations;
mod app;
mod camera;
mod cli;
mod evidence;
mod help;
mod render;
mod skill;
#[cfg(windows)]
mod spacemouse;
mod trail;
fn window_icon() -> Option<eframe::egui::IconData> {
    let ico = include_bytes!("../assets/cnc-viewer.ico");
    let count = u16::from_le_bytes(ico.get(4..6)?.try_into().ok()?) as usize;
    let entry = ico
        .get(6..6 + count * 16)?
        .chunks_exact(16)
        .find(|entry| entry[0] == 48)?;
    let length = u32::from_le_bytes(entry[8..12].try_into().ok()?) as usize;
    let offset = u32::from_le_bytes(entry[12..16].try_into().ok()?) as usize;
    let png = ico.get(offset..offset.checked_add(length)?)?;
    eframe::icon_data::from_png_bytes(png).ok()
}
fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if help::show(&args) {
        return;
    }
    match args.first().map(String::as_str) {
        Some("export-skill") => {
            let result = args
                .get(1)
                .ok_or_else(|| "Use export-skill DIRECTORY".into())
                .and_then(|p| skill::export(std::path::Path::new(p)));
            match result {
                Ok(()) => println!("{}", serde_json::json!({"skill":args[1]})),
                Err(error) => {
                    println!("{}", serde_json::json!({"error":error.to_string()}));
                    std::process::exit(1);
                }
            }
            return;
        }
        Some("inspect") => {
            let result = if let Some(file) = args.get(1) {
                cli::inspect(file, args.get(2).map(String::as_str))
            } else {
                Err("Use inspect FILE|- [JOB.json]".into())
            };
            match result {
                Ok((report, code)) => {
                    println!("{report}");
                    std::process::exit(code)
                }
                Err(error) => {
                    println!(
                        "{}",
                        serde_json::json!({"schema":"cnc-inspection-error/1","error":{"code":"INPUT_OR_ARTIFACT","message":error.to_string()}})
                    );
                    std::process::exit(1)
                }
            }
        }
        _ => {}
    }
    let receiving_tab = args.first().is_some_and(|arg| arg == "--tab-transfer");
    let file = if receiving_tab {
        None
    } else {
        args.first().map(std::path::PathBuf::from)
    };
    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_inner_size([1280., 800.])
        .with_min_inner_size([640., 400.]);
    if let Some(icon) = window_icon() {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions {
        viewport,
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    if let Err(error) = eframe::run_native(
        "CNC Viewer",
        options,
        Box::new(move |cc| Ok(Box::new(app::Window::new(cc, file, receiving_tab)?))),
    ) {
        eprintln!("{error}");
        std::process::exit(1)
    }
}
