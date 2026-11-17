#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use flip_lib::cli::{dispatch_from_args, DispatchAction};

fn main() {
    match dispatch_from_args(std::env::args_os()) {
        DispatchAction::Exit(code) => std::process::exit(code),
        DispatchAction::LaunchGui {
            initial_url,
            is_login_item,
        } => {
            flip_lib::run_with_options(initial_url, is_login_item);
        }
    }
}
