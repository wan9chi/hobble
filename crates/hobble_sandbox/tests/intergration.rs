use std::{fs::File, io::Write, path::PathBuf, process::Stdio};

use hobble_command_test::command_for_fn;
use tempfile::{NamedTempFile, tempdir};

#[test]
fn allow_path() {
    let mut profile = hobble_sandbox::SandboxProfile::default();

    let allowed_dir = tempdir().unwrap();
    let mut disallowed_file = NamedTempFile::new().unwrap();
    disallowed_file.write_all(b"disallowed").unwrap();

    let allowed_dir_path = allowed_dir.path().to_path_buf();
    let disallowed_path = disallowed_file.path().to_path_buf();
    profile.allowed_paths.push(allowed_dir_path.clone());

    let allowed_missing_path = allowed_dir_path.join("missing");
    let child_paths = format!(
        "{}\n{}",
        allowed_missing_path.display(),
        disallowed_path.display()
    );
    let mut command = command_for_fn!(child_paths, |child_paths: String| {
        let (allowed_path, disallowed_path) = child_paths.split_once('\n').unwrap();
        let allowed_path = PathBuf::from(allowed_path);
        let disallowed_path = PathBuf::from(disallowed_path);

        println!("open allowed: {}", open_result(allowed_path));
        println!("open disallowed: {}", open_result(disallowed_path));
    });
    command.current_dir(&allowed_dir_path);
    command.stdout(Stdio::piped()).stderr(Stdio::piped());

    let output = hobble_sandbox::spawn_with_sandbox(command, &profile)
        .unwrap()
        .wait_with_output()
        .unwrap();
    assert!(
        output.status.success(),
        "child failed with status {} stdout: {} stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = std::str::from_utf8(&output.stdout).unwrap();
    let stdout_lines = stdout.trim().lines().collect::<Vec<_>>();
    assert_eq!(
        stdout_lines,
        &[
            "open allowed: NotFound",
            "open disallowed: PermissionDenied",
        ]
    );
}

fn open_result(path: PathBuf) -> String {
    match File::open(path) {
        Ok(_) => "Ok".to_string(),
        Err(error) => format!("{:?}", error.kind()),
    }
}
