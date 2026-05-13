use anyhow::Context;
use assert_cmd::prelude::*;
use hyper::Client;
use std::collections::HashMap;
use std::fs;
use std::fs::File;
use std::path::Path;
use std::process::Command;
use std::time::Duration;
use tempdir::TempDir;
use tokio::signal;

fn read_folder_contents(folder_path: &Path) -> HashMap<String, String> {
    let mut contents = HashMap::new();

    let entries = fs::read_dir(folder_path).unwrap();
    for entry in entries {
        let path = entry.unwrap().path();
        if path.is_file() {
            let file_name = path.file_name().unwrap().to_str().unwrap().to_owned();
            let file_contents = fs::read_to_string(path).unwrap();
            contents.insert(file_name, file_contents);
        } else if path.is_dir() {
            let sub_folder_name = path.file_name().unwrap().to_str().unwrap().to_owned();
            let sub_folder_contents = read_folder_contents(&path);
            for (file_name, file_contents) in sub_folder_contents {
                let prefixed_file_name = format!("{}/{}", sub_folder_name, file_name);
                contents.insert(prefixed_file_name, file_contents);
            }
        }
    }

    contents
}

#[test]
fn test_creates_basic_output() {
    let tempdir = TempDir::new("output").unwrap();

    Command::cargo_bin("squid")
        .unwrap()
        .arg("--template-folder")
        .arg("tests/templates")
        .arg("--output-folder")
        .arg(tempdir.path())
        .arg("--markdown-folder")
        .arg("tests/markdown")
        .arg("--template-variables")
        .arg("tests/config.toml")
        .arg("--static-resources")
        .arg("tests/static")
        .assert()
        .success();

    let created = read_folder_contents(tempdir.path());
    let expected = read_folder_contents(Path::new("tests/output/"));

    assert!(!created.is_empty());
    for key in expected.keys() {
        if key == "rss.xml" {
            continue;
        }
        assert!(
            created.contains_key(key.as_str()),
            "expected file {key} was not created"
        );
    }
    for (key, value) in created {
        let expected_content = match expected.get(&key) {
            Some(t) => t,
            None => {
                println!("{value}");
                panic!("we were not expecting {key}");
            }
        };
        //FIXME: because the build time is always different, we need to mock the datetime
        //for the RSS
        if key == "rss.xml" {
            continue;
        }
        assert_eq!(expected_content, &value);
    }
}

#[tokio::test]
async fn test_watches() {
    let tempdir = TempDir::new("output").unwrap();
    let static_folder = TempDir::new("static_folder").unwrap();

    let output_folder = tempdir.path().to_str().to_owned().unwrap().to_string();
    let static_folder_cmd = static_folder
        .path()
        .to_str()
        .to_owned()
        .unwrap()
        .to_string();

    let cargo_bin = Command::cargo_bin("squid")
        .unwrap()
        .arg("--template-folder")
        .arg("tests/templates")
        .arg("--output-folder")
        .arg(output_folder)
        .arg("--markdown-folder")
        .arg("tests/markdown")
        .arg("--template-variables")
        .arg("tests/config.toml")
        .arg("--static-resources")
        .arg(static_folder_cmd)
        .arg("--watch")
        .spawn()
        .unwrap();

    File::create(static_folder.into_path().join("hello.txt")).unwrap();

    let result = tokio::time::timeout(Duration::from_secs(10), async {
        let path = tempdir.into_path().join("hello.txt");
        while !path.exists() {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        true
    })
    .await
    .context("file was not created before timeout of 10s")
    .unwrap();

    assert!(result);
    kill_child(&cargo_bin.id().to_string())
}

#[tokio::test]
async fn test_webserver() {
    let output_folder = TempDir::new("output").unwrap();

    let cargo_bin = Command::cargo_bin("squid")
        .unwrap()
        .arg("--template-folder")
        .arg("tests/templates")
        .arg("--output-folder")
        .arg(output_folder.path())
        .arg("--markdown-folder")
        .arg("tests/markdown")
        .arg("--template-variables")
        .arg("tests/config.toml")
        .arg("--static-resources")
        .arg("tests/static")
        .arg("--serve")
        .arg("8181")
        .spawn()
        .unwrap();

    let client = Client::new();
    let uri: hyper::Uri = "http://localhost:8181/index.html".parse().unwrap();

    let resp = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            match client.get(uri.clone()).await {
                Ok(r) => break r,
                Err(_) => tokio::time::sleep(Duration::from_millis(50)).await,
            }
        }
    })
    .await
    .context("server did not respond within 15s")
    .unwrap();

    assert_eq!(200, resp.status());

    kill_child(&cargo_bin.id().to_string())
}

fn kill_child(child_id: &str) {
    let mut kill = Command::new("kill")
        .args(["-s", "INT", child_id])
        .spawn()
        .unwrap();
    kill.wait().unwrap();
}
