//! Hauler's desktop window: a Kirigami front end for haulerd.

use cxx_qt_lib::{QQmlApplicationEngine, QQuickStyle, QString, QUrl};

mod app;
mod backend;
mod daemon;
mod single;

fn main() {
    // Already running: hand it our links (or have it show its window) and go.
    let demo = std::env::var_os("HAULER_DESKTOP_DEMO").is_some();
    if !demo && !single::claim(&single::links(std::env::args().skip(1))) {
        return;
    }
    cxx_qt::init_qml_module!("org.hauler.desktop");

    // Plasma provides the native Kirigami style; elsewhere fall back to Fusion,
    // unless the user picked a style themselves.
    if std::env::var_os("QT_QUICK_CONTROLS_STYLE").is_none() {
        let on_kde = std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|d| d.split(':').any(|p| p == "KDE"));
        QQuickStyle::set_style(&QString::from(if on_kde { "org.kde.desktop" } else { "Fusion" }));
    }

    let mut app = app::ffi::hauler_new_application(env!("CARGO_PKG_VERSION"));

    let mut engine = QQmlApplicationEngine::new();
    if let Some(engine) = engine.as_mut() {
        engine.load(&QUrl::from("qrc:/qt/qml/org/hauler/desktop/qml/Main.qml"));
    }
    if let Some(app) = app.as_mut() {
        let code = app::ffi::hauler_exec(app);
        daemon::stop_spawned_daemon();
        std::process::exit(code);
    }
}
