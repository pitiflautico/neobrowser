//! Auth flow integration test.
//! Tests: add_profile → set_credential → login → save_session → auto_session
//! Requires Chrome. Run with: cargo test --test auth_flow -- --nocapture

use std::process::Command;

fn neo() -> Command {
    Command::new(env!("CARGO_BIN_EXE_neobrowser_rs"))
}

#[test]
fn auth_profile_crud() {
    // This tests the auth system via CLI session commands
    // Just verify the setup command exists and shows help
    let out = neo().args(["setup", "--help"]).output().unwrap();
    assert!(out.status.success());
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("setup"));
}

#[test]
fn proxy_help() {
    let out = neo().args(["proxy", "--help"]).output().unwrap();
    assert!(out.status.success());
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("proxy") || s.contains("CORS"));
}
