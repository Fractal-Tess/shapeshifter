mod app;
mod self_check;
mod state;
mod views;

use app::ShapeshifterApp;

fn main() -> anyhow::Result<()> {
    if std::env::args().any(|arg| arg == "--self-check") {
        self_check::run()?;
        return Ok(());
    }

    let app = ShapeshifterApp::new()?;
    let options = native_options();
    eframe::run_native(
        "Shapeshifter",
        options,
        Box::new(move |_cc| Ok(Box::new(app))),
    )
    .map_err(|err| anyhow::anyhow!(err.to_string()))
}

fn native_options() -> eframe::NativeOptions {
    let mut options = eframe::NativeOptions::default();
    options.renderer = eframe::Renderer::Glow;
    #[cfg(target_os = "linux")]
    {
        use eframe::egui::ViewportBuilder;
        use winit::platform::x11::EventLoopBuilderExtX11;

        if std::env::var_os("DISPLAY").is_some() {
            options.viewport = ViewportBuilder::default().with_title("Shapeshifter");
            options.event_loop_builder = Some(Box::new(|builder| {
                builder.with_x11();
            }));
        }
    }
    options
}
