#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui;

/// Metal (through wgpu) on macOS; OpenGL elsewhere, which opens a window faster.
#[cfg(target_os = "macos")]
const RENDERER: eframe::Renderer = eframe::Renderer::Wgpu;
#[cfg(not(target_os = "macos"))]
const RENDERER: eframe::Renderer = eframe::Renderer::Glow;

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--version" | "-V") => {
            println!("TernaryCode {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        // The same as the separate `ternarycode-hook` program (which is what hooks run, since
        // it starts much faster than this one).
        Some("hook") => std::process::exit(tc_agent::client::run(&args[1..])),
        _ => {}
    }
    ternarycode::app::mark_process_start();
    ternarycode::platform::clean_inherited_env();
    install_crash_log();
    ternarycode::platform::raise_open_files_limit();
    let mut settings: tc_core::model::Settings =
        tc_core::persist::load_json(&ternarycode::paths::settings_file());
    settings.sanitize();
    let targets = ternarycode::instance::targets(args);
    // From a terminal, with no TernaryCode running: start it apart from the terminal.
    if !ternarycode::instance::is_running() && ternarycode::cli::launch_detached(&targets) {
        return Ok(());
    }
    let (lock, inbox) = match ternarycode::instance::acquire(&targets) {
        ternaryshell_like::Instance::Forwarded => return Ok(()),
        ternaryshell_like::Instance::Primary { lock, inbox } => (lock, inbox),
    };
    let open: Vec<std::path::PathBuf> = targets.into_iter().map(Into::into).collect();
    ternarycode::platform::disable_window_tabbing();
    let start = std::rc::Rc::new(std::cell::RefCell::new(Some((open, inbox))));
    // The app is created by whichever attempt opens a window.
    let attempt = |renderer: eframe::Renderer| {
        let start = start.clone();
        eframe::run_native(
            "TernaryCode",
            native_options(settings.window, renderer),
            Box::new(move |cc| {
                let (open, inbox) = start.borrow_mut().take().expect("the app starts once");
                Ok(Box::new(ternarycode::app::App::new(cc, open, inbox)))
            }),
        )
    };
    let result = attempt(RENDERER);
    // Windows without OpenGL 2 (a virtual machine, Remote Desktop, no graphics driver): Direct3D
    // 12, in software when there is no graphics card.
    #[cfg(windows)]
    let result = match result {
        Err(_) if start.borrow().is_some() => attempt(eframe::Renderer::Wgpu),
        other => other,
    };
    drop(lock);
    if let Err(e) = &result {
        report_start_failure(e);
    }
    result
}

/// A window that can't open is said in a dialog: on Windows and Linux nobody sees the console.
fn report_start_failure(error: &eframe::Error) {
    eprintln!("TernaryCode could not open its window: {error}");
    #[cfg(not(target_os = "macos"))]
    {
        let _ = rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Error)
            .set_title("TernaryCode")
            .set_description(format!("TernaryCode could not open its window:\n\n{error}"))
            .show();
    }
}

mod ternaryshell_like {
    pub use ternarycode::instance::Instance;
}

fn native_options(saved: Option<[f32; 4]>, renderer: eframe::Renderer) -> eframe::NativeOptions {
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("TernaryCode")
        .with_app_id(if cfg!(target_os = "linux") {
            "ternarycode"
        } else {
            "dev.ternarycode.app"
        })
        .with_inner_size([1280.0, 800.0])
        .with_min_inner_size([720.0, 440.0])
        .with_icon(window_icon());
    // macOS: the sidebar and header run under a transparent title bar (the window buttons sit
    // in the sidebar's top row; the header drags the window).
    #[cfg(target_os = "macos")]
    {
        viewport = viewport
            .with_fullsize_content_view(true)
            .with_titlebar_shown(false)
            .with_title_shown(false);
    }
    if let Some([x, y, w, h]) = saved {
        // A place far off the screens (a display unplugged since) is left to the system.
        if (-200.0..8000.0).contains(&x) && (0.0..8000.0).contains(&y) {
            viewport = viewport.with_position([x, y]);
        }
        if w >= 400.0 && h >= 300.0 {
            viewport = viewport.with_inner_size([w, h]);
        }
    }
    eframe::NativeOptions {
        viewport,
        #[cfg(target_os = "macos")]
        event_loop_builder: Some(Box::new(|builder| {
            use winit::platform::macos::EventLoopBuilderExtMacOS as _;
            builder.with_default_menu(false);
        })),
        renderer,
        // Two drawables instead of three: a window-sized buffer less (tens of MB on a Retina
        // display) and input shown a frame sooner. Frames here take about a millisecond, so
        // the extra queued frame bought no smoothness.
        #[cfg(target_os = "macos")]
        wgpu_options: eframe::egui_wgpu::WgpuConfiguration {
            surface: eframe::egui_wgpu::SurfaceConfig::LOW_LATENCY,
            ..Default::default()
        },
        ..Default::default()
    }
}

/// The window and taskbar icon. On macOS the .app bundle's icon is the Dock's: an empty icon
/// keeps it (without one, eframe would put its own logo in the Dock while the app runs).
fn window_icon() -> egui::IconData {
    if cfg!(target_os = "macos") {
        return egui::IconData::default();
    }
    eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon_256.png")).unwrap_or_default()
}

/// Writes a report to the logs folder when something panics, then runs the default hook.
fn install_crash_log() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let dir = ternarycode::paths::logs_dir();
        if std::fs::create_dir_all(&dir).is_ok() {
            let thread = std::thread::current()
                .name()
                .unwrap_or("unnamed")
                .to_string();
            let report = format!(
                "TernaryCode {} panicked on thread '{thread}' at {}\n{info}\n\n{}\n",
                env!("CARGO_PKG_VERSION"),
                chrono::Local::now().to_rfc3339(),
                std::backtrace::Backtrace::force_capture(),
            );
            let name = format!("panic-{}.log", chrono::Local::now().format("%Y%m%d-%H%M%S"));
            let _ = std::fs::write(dir.join(name), report);
        }
        default(info);
    }));
}
