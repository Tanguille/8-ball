use std::io::{BufRead, Read};

/// An `8ball serve` process with a fresh database, killed on drop.
pub struct Server {
    child: std::process::Child,
    pub url: String,
    stderr: Option<std::thread::JoinHandle<String>>,
    _directory: tempfile::TempDir,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(stderr) = self.stderr.take() {
            eprintln!("HTTP server stderr: {}", stderr.join().unwrap());
        }
    }
}

/// Starts `8ball serve` on a free local port. `args` holds the config, `["--config", path]` or
/// `["--config-toml", text]`, and any other flags such as `["--api-key", key]`.
#[must_use]
pub fn serve(args: &[&str]) -> Server {
    let directory = tempfile::Builder::new().prefix("8ball-").tempdir().unwrap();
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_8ball"));
    command
        .arg("serve")
        .args(args)
        .arg("--db")
        .arg(directory.path().join("db.sqlite"))
        .args(["--listen", "127.0.0.1:0"])
        // Config variables from the developer's shell would conflict with the config in `args`.
        .env_remove("EIGHTBALL_CONFIG")
        .env_remove("EIGHTBALL_CONFIG_TOML")
        .stderr(std::process::Stdio::piped());
    let mut server = Server {
        child: command.spawn().unwrap(),
        url: String::new(),
        stderr: None,
        _directory: directory,
    };
    let mut errors = String::new();
    let mut reader = std::io::BufReader::new(server.child.stderr.take().unwrap());
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap() == 0 {
            break;
        }
        if let Some((_, address)) = line.split_once("listening on ") {
            server.url = format!("http://{}", address.trim());
            // Keep reading, so a full stderr pipe never blocks the server.
            server.stderr = Some(std::thread::spawn(move || {
                let mut logs = String::new();
                let _ = reader.read_to_string(&mut logs);
                logs
            }));

            return server;
        }
        errors.push_str(&line);
    }

    panic!("HTTP server did not start: {errors}");
}
