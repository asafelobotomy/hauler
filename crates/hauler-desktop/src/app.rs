//! The Qt application object, created in C++ (see `cpp/app.cpp`).

#[cxx::bridge]
pub mod ffi {
    unsafe extern "C++" {
        include!("hauler-desktop/cpp/app.h");

        type QApplication;

        fn hauler_new_application(version: &str) -> UniquePtr<QApplication>;
        fn hauler_exec(app: Pin<&mut QApplication>) -> i32;
        fn hauler_grab_window(path: &str) -> bool;
    }
}
