//! Real-binary smoke tests. Everything else drives `gage_cli::run`
//! in-process; this file exists only for what needs an actual process.

use assert_cmd::cargo::cargo_bin_cmd;

#[test]
fn the_binary_builds_and_exits_zero() {
    cargo_bin_cmd!("gage").assert().success();
}
