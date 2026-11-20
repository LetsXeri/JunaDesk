use crate::{
    media,
    model::{Observation, Playback, Result},
};
use serde_json::json;
use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Stdio},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

pub struct Desktop {
    program: PathBuf,
    helper: Option<Helper>,
    retry_at: Instant,
}

struct Helper {
    child: Child,
    stdin: ChildStdin,
    latest: Arc<Mutex<Observation>>,
    desired: Option<BTreeMap<String, Playback>>,
}

impl Desktop {
    pub fn new(resources: &Path) -> Self {
        Self {
            program: media::binary(resources, "junadesk-wallpaper"),
            helper: None,
            retry_at: Instant::now(),
        }
    }

    pub fn observe(&mut self) -> Result<Observation> {
        if let Some(helper) = &mut self.helper {
            if matches!(helper.child.try_wait(), Ok(None)) {
                return Ok(helper.latest.lock().unwrap().clone());
            }
            self.helper = None;
        }
        if Instant::now() < self.retry_at {
            return Err("Die macOS-Wiedergabe wird neu gestartet.".into());
        }
        self.retry_at = Instant::now() + Duration::from_secs(5);
        self.helper = Some(Helper::start(&self.program)?);
        Ok(Observation::default())
    }

    pub fn apply(&mut self, desired: &BTreeMap<String, Playback>) -> Result<()> {
        let Some(helper) = &mut self.helper else {
            return Ok(());
        };
        if helper.desired.as_ref() == Some(desired) {
            return Ok(());
        }
        serde_json::to_writer(&mut helper.stdin, &json!({ "wallpapers": desired }))
            .map_err(|e| e.to_string())?;
        helper.stdin.write_all(b"\n").map_err(|e| e.to_string())?;
        helper.desired = Some(desired.clone());
        Ok(())
    }
}

impl Helper {
    fn start(program: &Path) -> Result<Self> {
        let mut child = media::command(program)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| format!("macOS-Wiedergabe konnte nicht gestartet werden: {e}"))?;
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let latest = Arc::new(Mutex::new(Observation::default()));
        let reader_state = latest.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout)
                .lines()
                .map_while(std::result::Result::ok)
            {
                if let Ok(observation) = serde_json::from_str(&line) {
                    *reader_state.lock().unwrap() = observation;
                }
            }
        });
        Ok(Self {
            child,
            stdin,
            latest,
            desired: None,
        })
    }
}

impl Drop for Helper {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
