use std::{
    env::home_dir,
    fs::{self, File},
    path::PathBuf,
    process::Stdio,
};

use hobble_command_test::command_for_fn;

#[test]
fn allow_path() {
    let mut builder = hobble_sandbox::SandboxBuilder::default();

    let run_id = uuid::Uuid::new_v4().to_string();
    let allowed_dirname = format!("allowed_{run_id}");
    let disallowed_filename = format!("disallowed_{run_id}");
    let home = home_dir().unwrap();
    let allowed_dir = home.join(&allowed_dirname);
    let disallowed_path = home.join(&disallowed_filename);
    let _cleanup = Cleanup {
        allowed_dir: allowed_dir.clone(),
        disallowed_path: disallowed_path.clone(),
    };

    fs::create_dir(&allowed_dir).unwrap();
    fs::write(&disallowed_path, b"disallowed").unwrap();

    builder.allow_path(allowed_dir.as_os_str());

    let allowed_missing_path = allowed_dir.join("missing");
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
    command.current_dir(&allowed_dir);
    command.stdout(Stdio::piped()).stderr(Stdio::piped());

    let output = builder.spawn(command).unwrap().wait_with_output().unwrap();
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

struct Cleanup {
    allowed_dir: PathBuf,
    disallowed_path: PathBuf,
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.disallowed_path);
        let _ = fs::remove_dir_all(&self.allowed_dir);
    }
}

fn open_result(path: PathBuf) -> String {
    match File::open(path) {
        Ok(_) => "Ok".to_string(),
        Err(error) => format!("{:?}", error.kind()),
    }
}
