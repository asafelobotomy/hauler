//! Hauler's desktop window: a Kirigami front end for haulerd.

use cxx_qt_lib::{QQmlApplicationEngine, QQuickStyle, QString, QUrl};

mod app;
mod backend;

fn main() {
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
        std::process::exit(app::ffi::hauler_exec(app));
    }
}
