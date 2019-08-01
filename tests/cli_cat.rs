// Copyright 2019 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

mod common;

#[macro_use]
extern crate duct;

use assert_cmd::prelude::*;
use common::{get_bin_location, parse_files_put_or_sync_output, CLI};
use predicates::prelude::*;
use std::process::Command;

const TEST_FILE: &str = "./tests/testfolder/test.md";
const TEST_FILE_CONTENT: &str = "hello tests!";
const ANOTHER_FILE: &str = "./tests/testfolder/another.md";
const ANOTHER_FILE_CONTENT: &str = "exists";

#[test]
fn calling_safe_cat() {
    let content = cmd!(get_bin_location(), "files", "put", TEST_FILE, "--json",)
        .read()
        .unwrap();

    let (_container_xorurl, map) = parse_files_put_or_sync_output(&content);
    let mut cmd = Command::cargo_bin(CLI).unwrap();
    cmd.args(&vec!["cat", &map[TEST_FILE].1])
        .assert()
        .stdout(predicate::str::contains(TEST_FILE_CONTENT))
        .success();
}

#[test]
fn calling_safe_cat_xorurl_url_with_version() {
    let content = cmd!(get_bin_location(), "files", "put", TEST_FILE, "--json",)
        .read()
        .unwrap();
    let (container_xorurl, _files_map) = parse_files_put_or_sync_output(&content);

    // let's sync with another file so we get a new version, and a different content in the file
    let xorurl_with_path = format!("{}/test.md", container_xorurl);
    let mut cmd = Command::cargo_bin(CLI).unwrap();
    cmd.args(&vec!["files", "sync", ANOTHER_FILE, &xorurl_with_path])
        .assert()
        .success();

    let mut cmd = Command::cargo_bin(CLI).unwrap();
    cmd.args(&vec!["cat", &xorurl_with_path])
        .assert()
        .stdout(predicate::str::contains(ANOTHER_FILE_CONTENT))
        .success();

    let v0_xorurl = format!("{}/test.md?v=0", container_xorurl);
    let mut cmd = Command::cargo_bin(CLI).unwrap();
    cmd.args(&vec!["cat", &v0_xorurl])
        .assert()
        .stdout(predicate::str::contains(TEST_FILE_CONTENT))
        .success();

    let v1_xorurl = format!("{}/test.md?v=1", container_xorurl);
    let mut cmd = Command::cargo_bin(CLI).unwrap();
    cmd.args(&vec!["cat", &v1_xorurl])
        .assert()
        .stdout(predicate::str::contains(ANOTHER_FILE_CONTENT))
        .success();

    let invalid_version_xorurl = format!("{}/test.md?v=2", container_xorurl);
    let mut cmd = Command::cargo_bin(CLI).unwrap();
    cmd.args(&vec!["cat", &invalid_version_xorurl])
        .assert()
        .failure();
}
