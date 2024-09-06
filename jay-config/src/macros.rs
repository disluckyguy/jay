/// Declares the entry point of the configuration.
#[macro_export]
macro_rules! config {
    ($f:path) => {
        #[no_mangle]
        #[used]
        pub static mut JAY_CONFIG_ENTRY_V1: $crate::_private::ConfigEntry = {
            struct X;
            impl $crate::_private::Config for X {
                extern "C" fn configure() {
                    $f();
                }
            }
            $crate::_private::ConfigEntryGen::<X>::ENTRY
        };
    };
}

macro_rules! try_get {
    () => {{
        unsafe {
            let client = crate::_private::client::CLIENT.with(|client| client.get());
            if client.is_null() {
                None
            } else {
                Some(&*client)
            }
        }
    }};
}

macro_rules! get {
    () => {{
        get!(Default::default())
    }};
    ($def:expr) => {{
        let client = unsafe {
            let client = crate::_private::client::CLIENT.with(|client| client.get());
            if client.is_null() {
                return $def;
            }
            &*client
        };
        client
    }};
}

// #[macro_export]
// macro_rules! log {
//     ($lvl:expr, $($arg:tt)+) => ({
//         $crate::log(
//             $lvl,
//             &format!($($args)*),
//         );
//     })
// }
//
// #[macro_export]
// macro_rules! trace {
//     ($($arg:tt)+) => {
//         $crate::log!($crate::LogLevel::Trace, $($arg)+)
//     }
// }
//
// #[macro_export]
// macro_rules! debug {
//     ($($arg:tt)+) => {
//         $crate::log!($crate::LogLevel::Debug, $($arg)+)
//     }
// }
//
// #[macro_export]
// macro_rules! info {
//     ($($arg:tt)+) => {
//         $crate::log!($crate::LogLevel::Info, $($arg)+)
//     }
// }
//
// #[macro_export]
// macro_rules! info {
//     ($($arg:tt)+) => {
//         $crate::log!($crate::LogLevel::Info, $($arg)+)
//     }
// }
