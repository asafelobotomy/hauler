use cxx_qt_build::{CxxQtBuilder, QmlFile, QmlModule};

fn main() {
    let builder = CxxQtBuilder::new_qml_module(QmlModule::new("org.hauler.desktop").qml_files([
        QmlFile::from("qml/Main.qml"),
        QmlFile::from("qml/Fmt.qml").singleton(true),
        QmlFile::from("qml/TaskModel.qml"),
        QmlFile::from("qml/TasksPage.qml"),
        QmlFile::from("qml/TaskDelegate.qml"),
        QmlFile::from("qml/SegmentMap.qml"),
        QmlFile::from("qml/DetailsDialog.qml"),
        QmlFile::from("qml/AddDialog.qml"),
        QmlFile::from("qml/SettingsPage.qml"),
    ]))
    .qt_module("Gui")
    .qt_module("Quick")
    .qt_module("QuickControls2")
    .qt_module("Widgets")
    .files(["src/backend.rs", "src/app.rs"])
    .cpp_file("cpp/app.cpp");
    // SAFETY: only adds a warning flag; nothing cxx-qt relies on changes.
    // GCC 16 warns about Qt's own headers here, which we cannot fix.
    let builder = unsafe {
        builder.cc_builder(|cc| {
            cc.flag_if_supported("-Wno-sfinae-incomplete");
        })
    };
    builder.build();
}
