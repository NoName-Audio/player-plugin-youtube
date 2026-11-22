use serde_json::{json, Value};
use std::{
    collections::{HashSet, VecDeque},
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{atomic::{AtomicBool, AtomicU64, Ordering}, mpsc, Arc},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const OUTPUT_LIMIT: usize = 1024 * 1024;
const ERROR_LIMIT: usize = 16 * 1024;
static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
pub enum Event {
    Search { key: String, result: Result<Vec<Value>, String> },
    Download { id: String, result: Result<PathBuf, String> },
    Progress { id: String, path: PathBuf },
    Cover { id: String, result: Result<PathBuf, String> },
}

struct Job {
    id: String,
    search: bool,
    cancel: Arc<AtomicBool>,
    thread: JoinHandle<()>,
}

pub struct Service {
    jobs: Vec<Job>,
    sender: mpsc::Sender<(Arc<AtomicBool>, Event)>,
    receiver: mpsc::Receiver<(Arc<AtomicBool>, Event)>,
    tokens: std::collections::HashMap<(bool, String), Arc<AtomicBool>>,
    covers: VecDeque<(String, String, String, PathBuf)>,
    cover_jobs: Vec<(Arc<AtomicBool>, JoinHandle<()>)>,
    cover_sender: mpsc::Sender<Event>,
    cover_receiver: mpsc::Receiver<Event>,
}

impl Default for Service {
    fn default() -> Self {
        let (sender, receiver) = mpsc::channel();
        let (cover_sender, cover_receiver) = mpsc::channel();
        Self { jobs: Vec::new(), sender, receiver, tokens: Default::default(), covers: Default::default(), cover_jobs: Vec::new(), cover_sender, cover_receiver }
    }
}

impl Service {
    pub fn cover(&mut self, id: String, url: String, executable: String, root: PathBuf) -> bool {
        if self.covers.len() >= 40 { return false; }
        self.covers.push_back((id, url, executable, root));
        self.start_covers();
        true
    }

    fn start_covers(&mut self) {
        let mut index = 0;
        while index < self.cover_jobs.len() {
            if self.cover_jobs[index].1.is_finished() {
                let (_, worker) = self.cover_jobs.swap_remove(index);
                let _ = worker.join();
            } else { index += 1; }
        }
        while self.cover_jobs.len() < 2 {
            let Some((id, url, executable, root)) = self.covers.pop_front() else { break };
            let cancel = Arc::new(AtomicBool::new(false));
            let signal = cancel.clone();
            let sender = self.cover_sender.clone();
            let failed_id = id.clone();
            match thread::Builder::new().name("youtube-cover".into()).spawn(move || {
                let result = download_cover(&url, &executable, &root, &signal);
                let _ = sender.send(Event::Cover { id, result });
            }) {
                Ok(worker) => self.cover_jobs.push((cancel, worker)),
                Err(error) => { let _ = self.cover_sender.send(Event::Cover { id: failed_id, result: Err(error.to_string()) }); }
            }
        }
    }
    pub fn search(&mut self, key: String, query: String, executable: String) -> Result<(), String> {
        let target = search_target(&query)?;
        self.reap();
        self.tokens.retain(|(search, _), token| {
            if *search { token.store(true, Ordering::Relaxed); }
            !*search
        });
        for job in &self.jobs {
            if job.search { job.cancel.store(true, Ordering::Relaxed); }
        }
        // A cancelled process still counts until its worker has reaped it.
        if self.jobs.iter().filter(|j| j.search).count() >= 2 {
            return Err("Previous YouTube search is still stopping; try again shortly".into());
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let signal = cancel.clone();
        let sender = self.sender.clone();
        let thread_key = key.clone();
        let worker = thread::Builder::new().name("youtube-search".into()).spawn(move || {
            let mut command = command(&executable);
            command.args(["--flat-playlist", "--dump-single-json", "--skip-download", "--", &target]);
            let result = run(command, &signal, Duration::from_secs(60)).and_then(|data| parse_search(&data));
            if !signal.load(Ordering::Relaxed) {
                let _ = sender.send((signal.clone(), Event::Search { key: thread_key, result }));
            }
        }).map_err(|e| e.to_string())?;
        self.tokens.insert((true, key.clone()), cancel.clone());
        self.jobs.push(Job { id: key, search: true, cancel, thread: worker });
        Ok(())
    }

    pub fn download(&mut self, id: String, url: String, executable: String, root: PathBuf, options: DownloadOptions) -> Result<(), String> {
        let url = youtube_url(&url)?;
        self.reap();
        if self.jobs.iter().filter(|j| !j.search).count() >= 2 {
            return Err("Two YouTube downloads are already running".into());
        }
        if self.jobs.iter().any(|j| !j.search && j.id == id) {
            return Err("This YouTube download is already running".into());
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let signal = cancel.clone();
        let sender = self.sender.clone();
        let thread_id = id.clone();
        let worker = thread::Builder::new().name("youtube-download".into()).spawn(move || {
            let result = download_file(&url, &executable, &root, &signal, options, |path| {
                let _ = sender.send((signal.clone(), Event::Progress { id: thread_id.clone(), path }));
            });
            let _ = sender.send((signal.clone(), Event::Download { id: thread_id, result }));
        }).map_err(|e| e.to_string())?;
        self.tokens.insert((false, id.clone()), cancel.clone());
        self.jobs.push(Job { id, search: false, cancel, thread: worker });
        Ok(())
    }

    pub fn cancel(&mut self, id: &str) {
        self.tokens.retain(|(_, key), token| {
            if key == id { token.store(true, Ordering::Relaxed); }
            key != id
        });
        for job in &self.jobs {
            if job.id == id { job.cancel.store(true, Ordering::Relaxed); }
        }
    }

    pub fn drain(&mut self) -> Vec<Event> {
        self.reap();
        self.start_covers();
        let mut events: Vec<_> = self.receiver.try_iter().filter_map(|(token, event)| {
            let key = match &event {
                Event::Search { key, .. } => (true, key.clone()),
                Event::Download { id, .. } => (false, id.clone()),
                Event::Progress { id, .. } => (false, id.clone()),
                Event::Cover { .. } => unreachable!(),
            };
            let current = self.tokens.get(&key).is_some_and(|active| Arc::ptr_eq(active, &token));
            if current && !matches!(&event, Event::Progress { .. }) { self.tokens.remove(&key); }
            (current && !token.load(Ordering::Relaxed)).then_some(event)
        }).collect();
        events.extend(self.cover_receiver.try_iter());
        events
    }

    fn reap(&mut self) {
        let mut i = 0;
        while i < self.jobs.len() {
            if self.jobs[i].thread.is_finished() {
                let _ = self.jobs.swap_remove(i).thread.join();
            } else { i += 1; }
        }
    }

    pub fn stop(&mut self) {
        self.covers.clear();
        for (cancel, _) in &self.cover_jobs { cancel.store(true, Ordering::Relaxed); }
        for (_, worker) in self.cover_jobs.drain(..) { let _ = worker.join(); }
        while let Ok(event) = self.cover_receiver.try_recv() {
            if let Event::Cover { result: Ok(path), .. } = event {
                if let Some(parent) = path.parent() { let _ = fs::remove_dir_all(parent); }
            }
        }
        for job in &self.jobs { job.cancel.store(true, Ordering::Relaxed); }
        for job in self.jobs.drain(..) { let _ = job.thread.join(); }
        self.tokens.clear();
        while self.receiver.try_recv().is_ok() {}
    }
}

impl Drop for Service {
    fn drop(&mut self) { self.stop(); }
}

fn command(executable: &str) -> Command {
    let mut command = Command::new(executable);
    command.args([
        "--ignore-config", "--no-plugin-dirs", "--no-playlist", "--no-progress",
        "--no-colors", "--no-warnings", "--socket-timeout", "15", "--retries", "2",
        "--fragment-retries", "2", "--no-cache-dir",
    ]);
    let deno = Path::new(executable).with_file_name(if cfg!(windows) { "deno.exe" } else { "deno" });
    let mut runtime = std::ffi::OsString::from("deno:");
    runtime.push(deno);
    command.args(["--no-js-runtimes", "--js-runtimes"]).arg(runtime);
    command.arg("--no-remote-components");
    command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    command
}

// Always keep draining both pipes, even once the retained output is full.
fn capture(mut input: impl Read, limit: usize, overflow: Arc<AtomicBool>) -> io::Result<Vec<u8>> {
    let mut result = Vec::new();
    let mut buffer = [0u8; 8192];
    loop {
        let count = match input.read(&mut buffer) {
            Ok(0) => return Ok(result),
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        let keep = count.min(limit.saturating_sub(result.len()));
        result.extend_from_slice(&buffer[..keep]);
        if keep < count { overflow.store(true, Ordering::Relaxed); }
    }
}

fn run(command: Command, cancel: &AtomicBool, timeout: Duration) -> Result<Vec<u8>, String> {
    run_observing(command, cancel, timeout, || {})
}

fn run_observing(mut command: Command, cancel: &AtomicBool, timeout: Duration, mut observe: impl FnMut()) -> Result<Vec<u8>, String> {
    if cancel.load(Ordering::Relaxed) { return Err("Download cancelled".into()); }
    let mut child = command.spawn().map_err(|e| format!("Cannot start yt-dlp (check its executable path): {e}"))?;
    let overflow = Arc::new(AtomicBool::new(false));
    let stdout = child.stdout.take().ok_or("Missing yt-dlp stdout")?;
    let stderr = child.stderr.take().ok_or("Missing yt-dlp stderr")?;
    let out_limit = overflow.clone();
    let out = thread::spawn(move || capture(stdout, OUTPUT_LIMIT, out_limit));
    let err = thread::spawn(move || capture(stderr, ERROR_LIMIT, Arc::new(AtomicBool::new(false))));
    let deadline = Instant::now() + timeout;
    let status = loop {
        let reason = if cancel.load(Ordering::Relaxed) { Some("Download cancelled") }
            else if overflow.load(Ordering::Relaxed) { Some("yt-dlp output exceeded 1 MiB") }
            else if Instant::now() >= deadline { Some("yt-dlp timed out") }
            else { None };
        if let Some(reason) = reason {
            let _ = child.kill();
            let _ = child.wait();
            break Err(reason.to_string());
        }
        observe();
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => thread::sleep(Duration::from_millis(25)),
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(e.to_string());
            }
        }
    };
    let output = out.join().map_err(|_| "yt-dlp output reader failed")?.map_err(|e| e.to_string())?;
    let error = err.join().map_err(|_| "yt-dlp error reader failed")?.map_err(|e| e.to_string())?;
    let status = status?;
    if overflow.load(Ordering::Relaxed) { return Err("yt-dlp output exceeded 1 MiB".into()); }
    if !status.success() {
        let message = String::from_utf8_lossy(&error);
        return Err(format!("yt-dlp failed ({status}): {}", message.trim()));
    }
    Ok(output)
}

fn video_id(id: &str) -> bool {
    id.len() == 11 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn youtube_url(input: &str) -> Result<String, String> {
    let input = input.trim();
    let rest = input.strip_prefix("https://").or_else(|| input.strip_prefix("http://"))
        .ok_or("Only YouTube http(s) video URLs are supported")?;
    let (host, path) = rest.split_once('/').ok_or("A YouTube video URL is required")?;
    let id = match host.to_ascii_lowercase().as_str() {
        "youtu.be" | "www.youtu.be" => path.split(['?', '#', '/']).next().unwrap_or(""),
        "youtube.com" | "www.youtube.com" | "m.youtube.com" | "music.youtube.com" => {
            if let Some(query) = path.strip_prefix("watch?") {
                query.split('#').next().unwrap_or("").split('&')
                    .find_map(|part| part.strip_prefix("v=")).unwrap_or("")
            } else {
                path.strip_prefix("shorts/").or_else(|| path.strip_prefix("live/"))
                    .or_else(|| path.strip_prefix("embed/"))
                    .unwrap_or("").split(['?', '#', '/']).next().unwrap_or("")
            }
        }
        _ => return Err("Only YouTube video URLs are supported".into()),
    };
    if !video_id(id) { return Err("A valid YouTube video URL is required (not a playlist)".into()); }
    Ok(format!("https://www.youtube.com/watch?v={id}"))
}

fn search_target(query: &str) -> Result<String, String> {
    let query = query.trim();
    if query.is_empty() { return Err("Enter a YouTube search or video URL".into()); }
    if query.len() > 2048 { return Err("Search is too long".into()); }
    if query.contains("://") || query.starts_with("www.") || query.starts_with("youtu.be/") {
        return youtube_url(query);
    }
    Ok(format!("ytsearch20:{query}"))
}

fn parse_search(data: &[u8]) -> Result<Vec<Value>, String> {
    let value: Value = serde_json::from_slice(data).map_err(|e| format!("Invalid yt-dlp JSON: {e}"))?;
    let entries: Vec<&Value> = match value.get("entries") {
        Some(Value::Array(entries)) => entries.iter().take(20).collect(),
        Some(_) => return Err("Invalid yt-dlp search entries".into()),
        None => vec![&value],
    };
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for entry in entries {
        let Some(id) = entry.get("id").and_then(Value::as_str).filter(|id| video_id(id)) else { continue };
        // Search is explicitly YouTube; reject a contradictory extractor URL.
        if let Some(url) = entry.get("webpage_url").or_else(|| entry.get("url")).and_then(Value::as_str) {
            if url.contains("://") && youtube_url(url).is_err() { continue; }
        }
        if !seen.insert(id) { continue; }
        let title = entry.get("title").and_then(Value::as_str).unwrap_or(id);
        let artist = ["artist", "uploader", "channel"].iter()
            .find_map(|field| entry.get(*field).and_then(Value::as_str)).unwrap_or("YouTube");
        let duration = entry.get("duration").and_then(Value::as_f64).filter(|d| d.is_finite() && *d >= 0.0);
        let thumbnail = entry.get("thumbnail").and_then(Value::as_str)
            .or_else(|| entry.get("thumbnails")?.as_array()?.iter().rev().find_map(|t| t["url"].as_str()))
            .filter(|url| url.starts_with("https://"));
        result.push(json!({"id": id, "title": title, "artist": artist, "duration": duration,
            "url": format!("https://www.youtube.com/watch?v={id}"), "thumbnail": thumbnail}));
    }
    Ok(result)
}

fn staging_directory(root: &Path) -> Result<PathBuf, String> {
    fs::create_dir_all(root).map_err(|e| format!("Cannot create download directory: {e}"))?;
    let root = fs::canonicalize(root).map_err(|e| e.to_string())?;
    for _ in 0..100 {
        let serial = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = root.join(format!("youtube-{}-{serial}", std::process::id()));
        match fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("Cannot create download staging directory: {e}")),
        }
    }
    Err("Cannot allocate a unique download directory".into())
}

fn final_path(stage: &Path, data: &[u8]) -> Result<PathBuf, String> {
    let text = std::str::from_utf8(data).map_err(|_| "Invalid yt-dlp output path")?;
    let paths: Vec<&str> = text.lines().filter(|line| !line.trim().is_empty()).collect();
    if paths.len() != 1 { return Err("yt-dlp did not report exactly one audio file".into()); }
    let path = PathBuf::from(paths[0]);
    if path.parent() != Some(stage) { return Err("yt-dlp reported a path outside its download directory".into()); }
    let metadata = fs::symlink_metadata(&path).map_err(|e| format!("Missing downloaded audio: {e}"))?;
    if !metadata.file_type().is_file() || metadata.len() == 0 {
        return Err("Downloaded audio is not a nonempty regular file".into());
    }
    let canonical = fs::canonicalize(&path).map_err(|e| e.to_string())?;
    if canonical.parent() != Some(stage) { return Err("Downloaded audio escaped its download directory".into()); }
    if !matches!(canonical.extension().and_then(|e| e.to_str()), Some("m4a" | "webm" | "opus" | "mp4")) {
        return Err("yt-dlp returned an unsupported audio container".into());
    }
    Ok(canonical)
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct DownloadOptions {
    pub video: bool,
    pub save_data: bool,
}

fn download_file(url: &str, executable: &str, root: &Path, cancel: &AtomicBool, options: DownloadOptions, mut ready: impl FnMut(PathBuf)) -> Result<PathBuf, String> {
    if cancel.load(Ordering::Relaxed) { return Err("Download cancelled".into()); }
    let stage = staging_directory(root)?;
    let (filename, format) = match (options.video, options.save_data) {
        (true, false) => ("video.mp4", "best[ext=mp4][vcodec^=avc1][acodec^=mp4a]"),
        (true, true) => ("video.mp4", "worst[ext=mp4][vcodec^=avc1][acodec^=mp4a]"),
        (false, false) => ("audio.webm", "bestaudio[ext=webm][acodec=opus][vcodec=none]"),
        (false, true) => ("audio.webm", "worstaudio[ext=webm][acodec=opus][vcodec=none]"),
    };
    let path = stage.join(filename);
    let result = (|| {
        let mut command = command(executable);
        // WebM carries initialization before its audio clusters. Ordinary M4A may
        // put its moov box at EOF and must never be advertised as a growing file.
        // No conversion, merging, external downloader or temporary .part rename.
        command.args(["--format", format,
            "--fixup", "never", "--no-simulate", "--no-keep-video", "--no-part",
            "--concurrent-fragments", "1", "--print", "after_move:filepath", "--paths"]);
        command.arg(&stage).args(["--output", filename, "--", url]);
        let mut announced = false;
        let data = run_observing(command, cancel, Duration::from_secs(15 * 60), || {
            if !options.video && !announced && playable_prefix(&path) {
                announced = true;
                ready(path.clone());
            }
        })?;
        if cancel.load(Ordering::Relaxed) { return Err("Download cancelled".into()); }
        let completed = final_path(&stage, &data)?;
        if completed != path { return Err("yt-dlp changed the requested output path".into()); }
        Ok(completed)
    })();
    if result.is_err() { let _ = fs::remove_dir_all(&stage); }
    result
}

fn playable_prefix(path: &Path) -> bool {
    let Ok(metadata) = fs::symlink_metadata(path) else { return false };
    if !metadata.file_type().is_file() || metadata.len() < 384 * 1024 { return false; }
    let Ok(mut file) = fs::File::open(path) else { return false };
    // Initialization is small; inspect a bounded prefix, never the whole growing file.
    let mut prefix = [0u8; 64 * 1024];
    let Ok(count) = file.read(&mut prefix) else { return false };
    let prefix = &prefix[..count];
    prefix.starts_with(&[0x1a, 0x45, 0xdf, 0xa3])
        && prefix.windows(8).any(|w| w == b"OpusHead")
        && prefix.windows(4).any(|w| w == [0x1f, 0x43, 0xb6, 0x75])
}

fn download_cover(url: &str, executable: &str, root: &Path, cancel: &AtomicBool) -> Result<PathBuf, String> {
    let url = youtube_url(url)?;
    let stage = staging_directory(root)?;
    let result = (|| {
        let mut process = command(executable);
        process.args(["--skip-download", "--write-thumbnail", "--paths"])
            .arg(&stage).args(["--output", "cover.%(ext)s", "--", &url]);
        run(process, cancel, Duration::from_secs(45))?;
        if cancel.load(Ordering::Relaxed) { return Err("Cover download cancelled".into()); }
        let mut covers = Vec::new();
        for entry in fs::read_dir(&stage).map_err(|e| e.to_string())? {
            let path = entry.map_err(|e| e.to_string())?.path();
            let metadata = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
            if metadata.file_type().is_file() && metadata.len() > 0 && metadata.len() <= 16 * 1024 * 1024
                && path.file_stem().and_then(|s| s.to_str()) == Some("cover")
                && matches!(path.extension().and_then(|s| s.to_str()), Some("jpg" | "jpeg" | "png" | "webp")) {
                covers.push(path);
            }
        }
        if covers.len() != 1 { return Err("yt-dlp did not download one supported thumbnail".into()); }
        Ok(covers.remove(0))
    })();
    if result.is_err() { let _ = fs::remove_dir_all(&stage); }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    #[test]
    fn cancellation_during_progress_reaps_process() {
        let mut process = Command::new("/bin/sh");
        process.args(["-c", "while :; do :; done"])
            .stdout(Stdio::piped()).stderr(Stdio::piped());
        let cancel = AtomicBool::new(false);
        let error = run_observing(process, &cancel, Duration::from_secs(2), || {
            cancel.store(true, Ordering::Relaxed);
        }).unwrap_err();
        assert!(error.contains("cancelled"));
    }
    #[cfg(unix)]
    #[test]
    fn data_saving_video_uses_muxed_mp4_without_progressive_preview() {
        use std::os::unix::fs::PermissionsExt;
        let root = staging_directory(&std::env::temp_dir()).unwrap();
        let script = root.join("yt-dlp");
        fs::write(&script, r#"#!/bin/sh
set -eu
while [ "$#" -gt 0 ]; do
    case "$1" in
        --format) shift; test "$1" = 'worst[ext=mp4][vcodec^=avc1][acodec^=mp4a]' ;;
        --paths) shift; stage=$1 ;;
        --output) shift; output=$1 ;;
    esac
    shift
done
printf 'fixture' > "$stage/$output"
printf '%s/%s\n' "$stage" "$output"
"#).unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o700)).unwrap();
        let result = download_file("https://www.youtube.com/watch?v=dQw4w9WgXcQ", script.to_str().unwrap(), &root, &AtomicBool::new(false), DownloadOptions { video: true, save_data: true }, |_| panic!("video must wait for completion"));
        fs::remove_dir_all(root).unwrap();
        assert_eq!(result.unwrap().file_name().unwrap(), "video.mp4");
    }

    #[cfg(unix)]
    #[test]
    fn progressive_webm_is_ready_before_downloader_exits() {
        use std::os::unix::fs::PermissionsExt;
        let root = staging_directory(&std::env::temp_dir()).unwrap();
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/stream-test.webm");
        let release = root.join("release");
        let ended = root.join("ended");
        let script = root.join("yt-dlp");
        fs::write(&script, format!("#!/bin/sh\nwhile [ $# -gt 0 ]; do if [ \"$1\" = --paths ]; then shift; stage=$1; fi; shift; done\ndd if='{}' of=\"$stage/audio.webm\" bs=1024 count=384 2>/dev/null\nwhile [ ! -f '{}' ]; do :; done\ndd if='{}' bs=1024 skip=384 2>/dev/null >> \"$stage/audio.webm\"\nprintf done > '{}'\nprintf '%s/audio.webm\\n' \"$stage\"\n", fixture.display(), release.display(), fixture.display(), ended.display())).unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o700)).unwrap();
        let mut early = None;
        let completed = download_file("https://www.youtube.com/watch?v=dQw4w9WgXcQ", script.to_str().unwrap(), &root, &AtomicBool::new(false), DownloadOptions::default(), |path| {
            assert!(!ended.exists(), "preview must precede process completion");
            assert!(playable_prefix(&path));
            assert_eq!(fs::metadata(&path).unwrap().len(), 384 * 1024);
            early = Some(path);
            fs::write(&release, b"continue").unwrap();
        }).unwrap();
        assert_eq!(early.as_ref(), Some(&completed));
        assert_eq!(fs::read(completed).unwrap(), fs::read(fixture).unwrap());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn progress_does_not_consume_terminal_generation() {
        let mut service = Service::default();
        let token = Arc::new(AtomicBool::new(false));
        service.tokens.insert((false, "video".into()), token.clone());
        service.sender.send((token.clone(), Event::Progress { id: "video".into(), path: "audio.webm".into() })).unwrap();
        service.sender.send((token, Event::Download { id: "video".into(), result: Ok("audio.webm".into()) })).unwrap();
        let events = service.drain();
        assert!(matches!(&events[..], [Event::Progress { .. }, Event::Download { .. }]));
        assert!(service.tokens.is_empty());
    }

    #[test]
    fn growing_prefix_rejects_incomplete_or_wrong_container() {
        let root = staging_directory(&std::env::temp_dir()).unwrap();
        let path = root.join("audio.webm");
        let fixture = include_bytes!("../assets/stream-test.webm");
        fs::write(&path, &fixture[..1024]).unwrap();
        assert!(!playable_prefix(&path));
        fs::write(&path, vec![0; 384 * 1024]).unwrap();
        assert!(!playable_prefix(&path));
        fs::write(&path, &fixture[..384 * 1024]).unwrap();
        assert!(playable_prefix(&path));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn parser_extracts_real_thumbnail_metadata() {
        let row = json!({"id":"dQw4w9WgXcQ","thumbnails":[{"url":"https://i.ytimg.com/first.jpg"},{"url":"https://i.ytimg.com/best.webp"}]});
        assert_eq!(parse_search(&serde_json::to_vec(&row).unwrap()).unwrap()[0]["thumbnail"], "https://i.ytimg.com/best.webp");
    }
    #[test]
    fn command_uses_private_runtime_and_no_external_components() {
        let executable = std::env::temp_dir().join("private runtimes").join(if cfg!(windows) { "yt-dlp.exe" } else { "yt-dlp" });
        let command = command(executable.to_str().unwrap());
        let args: Vec<_> = command.get_args().map(|arg| arg.to_string_lossy().into_owned()).collect();
        assert_eq!(command.get_program(), executable.as_os_str());
        let index = args.iter().position(|arg| arg == "--js-runtimes").unwrap();
        assert_eq!(args[index + 1], format!("deno:{}", executable.with_file_name(if cfg!(windows) { "deno.exe" } else { "deno" }).display()));
        for flag in ["--ignore-config", "--no-plugin-dirs", "--no-js-runtimes", "--no-remote-components"] { assert!(args.iter().any(|arg| arg == flag)); }
    }

    #[test]
    fn queued_cancelled_generation_cannot_consume_retry() {
        let mut service = Service::default();
        let old = Arc::new(AtomicBool::new(false));
        service.tokens.insert((false, "video".into()), old.clone());
        service.sender.send((old, Event::Download { id: "video".into(), result: Err("old".into()) })).unwrap();
        service.cancel("video");
        let new = Arc::new(AtomicBool::new(false));
        service.tokens.insert((false, "video".into()), new.clone());
        service.sender.send((new, Event::Download { id: "video".into(), result: Err("new".into()) })).unwrap();
        let events = service.drain();
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], Event::Download { result: Err(error), .. } if error == "new"));
    }

    #[test]
    fn urls_are_canonical_and_provider_restricted() {
        assert_eq!(youtube_url("https://youtu.be/dQw4w9WgXcQ?list=foo").unwrap(), "https://www.youtube.com/watch?v=dQw4w9WgXcQ");
        for url in ["--exec=bad", "https://youtube.com.evil/watch?v=dQw4w9WgXcQ", "https://youtube.com@evil/watch?v=dQw4w9WgXcQ", "file:///tmp/a", "https://youtube.com/playlist?list=x", "https://vimeo.com/dQw4w9WgXcQ"] {
            assert!(youtube_url(url).is_err(), "{url}");
        }
        assert_eq!(search_target("--exec bad").unwrap(), "ytsearch20:--exec bad");
        assert!(search_target("https://vimeo.com/123").is_err());
    }

    #[test]
    fn parser_handles_direct_search_duplicates_and_bad_rows() {
        let row = json!({"id":"dQw4w9WgXcQ", "title":"Title", "uploader":"Artist", "duration":12.5});
        assert_eq!(parse_search(&serde_json::to_vec(&row).unwrap()).unwrap()[0]["artist"], "Artist");
        let data = json!({"entries":[row.clone(), row, null, {"id":"invalid"}, {"id":"abcdefghijk", "url":"https://vimeo.com/x"}]});
        let rows = parse_search(&serde_json::to_vec(&data).unwrap()).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["duration"], 12.5);
        assert!(parse_search(b"not json").is_err());
        assert!(parse_search(br#"{"entries":null}"#).is_err());
    }

    #[test]
    fn capture_bounds_retention_but_drains_input() {
        let mut input = io::Cursor::new(vec![b'x'; OUTPUT_LIMIT + 4096]);
        let overflow = Arc::new(AtomicBool::new(false));
        let data = capture(&mut input, OUTPUT_LIMIT, overflow.clone()).unwrap();
        assert_eq!(data.len(), OUTPUT_LIMIT);
        assert!(overflow.load(Ordering::Relaxed));
        assert_eq!(input.position(), (OUTPUT_LIMIT + 4096) as u64);
    }

    #[test]
    fn final_paths_reject_escape_empty_and_symlinks() {
        let stage = staging_directory(&std::env::temp_dir()).unwrap();
        let audio = stage.join("audio.m4a");
        fs::write(&audio, b"audio").unwrap();
        assert_eq!(final_path(&stage, audio.to_str().unwrap().as_bytes()).unwrap(), audio);
        assert!(final_path(&stage, b"/tmp/escape.m4a").is_err());
        assert!(final_path(&stage, b"one\ntwo\n").is_err());
        fs::write(&audio, b"").unwrap();
        assert!(final_path(&stage, audio.to_str().unwrap().as_bytes()).is_err());
        #[cfg(unix)] {
            let link = stage.join("link.m4a");
            std::os::unix::fs::symlink(&audio, &link).unwrap();
            assert!(final_path(&stage, link.to_str().unwrap().as_bytes()).is_err());
        }
        fs::remove_dir_all(stage).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn process_drains_stderr_and_enforces_deadline_and_cancel() {
        // Shell is a test fixture only; production always invokes yt-dlp directly.
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "i=0; while [ $i -lt 20000 ]; do printf 'stderr fixture line\\n' >&2; i=$((i+1)); done; printf '{}' "])
            .stdout(Stdio::piped()).stderr(Stdio::piped());
        assert_eq!(run(command, &AtomicBool::new(false), Duration::from_secs(10)).unwrap(), b"{}");
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "while :; do :; done"]).stdout(Stdio::piped()).stderr(Stdio::piped());
        assert!(run(command, &AtomicBool::new(false), Duration::from_millis(50)).unwrap_err().contains("timed out"));
        let command = Command::new("/definitely/not/a/program");
        assert!(run(command, &AtomicBool::new(true), Duration::from_secs(1)).unwrap_err().contains("cancelled"));
    }
}
