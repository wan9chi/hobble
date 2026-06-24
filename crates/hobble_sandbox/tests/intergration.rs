use std::{env::home_dir, fs::File};

use hobble_command_test::command_for_fn;

#[test]
fn allow_path() {
    let mut builder = hobble_sandbox::SandboxBuilder::default();

    let allowed_filename = format!("allowed_{}", uuid::Uuid::new_v4());

    let allowed_path = home_dir().unwrap().join(&allowed_filename);
    builder.allow_path(allowed_path.as_os_str());

    let mut command = command_for_fn!(allowed_filename, |allowed_filename: String| {
        let allowed_path = home_dir().unwrap().join(allowed_filename);
        let disallowed_path = home_dir().unwrap().join("disallowed");

        println!("open allowed: {:?}", File::open(allowed_path).err().unwrap().kind());
        println!("open disallowed: {:?}", File::open(disallowed_path).err().unwrap().kind());
    });

    let output = command.output().unwrap();
    let stdout = std::str::from_utf8(&output.stdout).unwrap();
    let stdout_lines = stdout.trim().lines().collect::<Vec<_>>();
    assert_eq!(stdout_lines, &[
        "open allowed: NotFound",
        "open disallowed: NotFound",
    ]);
}
