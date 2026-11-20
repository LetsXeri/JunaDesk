use crate::model::{Result, Wallpaper};
use std::{
    ffi::{OsStr, OsString},
    fs,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};
use tempfile::{NamedTempFile, TempDir};
use uuid::Uuid;

#[derive(Clone)]
pub struct Tools {
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
}

pub fn command(program: &Path) -> Command {
    let mut command = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    command.current_dir(std::env::temp_dir());
    command
}

pub fn binary(resources: &Path, name: &str) -> PathBuf {
    let filename = format!("{name}{}", std::env::consts::EXE_SUFFIX);
    let bundled = resources.join(&filename);
    if bundled.is_file() {
        return bundled;
    }
    #[cfg(debug_assertions)]
    {
        let local = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("resources")
            .join(&filename);
        if local.is_file() {
            return local;
        }
        return PathBuf::from(filename);
    }
    #[cfg(not(debug_assertions))]
    bundled
}

fn output(program: &Path, args: impl IntoIterator<Item = impl AsRef<OsStr>>) -> Result<Output> {
    let output = command(program)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|error| {
            format!(
                "{} konnte nicht gestartet werden: {error}",
                program.display()
            )
        })?;
    if output.status.success() {
        return Ok(output);
    }
    let message = String::from_utf8_lossy(&output.stderr);
    Err(message
        .lines()
        .filter(|line| !line.trim().is_empty())
        .last()
        .unwrap_or("Die Mediendatei konnte nicht verarbeitet werden.")
        .into())
}

pub fn rendition(directory: &Path, fps: Option<u8>) -> PathBuf {
    directory.join(match fps {
        Some(15) => "video-15.mp4",
        Some(30) => "video-30.mp4",
        _ => "video.mp4",
    })
}

fn encode(tools: &Tools, source: &Path, path: &Path, fps: Option<u8>) -> Result<()> {
    let mut filters = "scale=w='min(iw,3840)':h='min(ih,3840)':force_original_aspect_ratio=decrease:force_divisible_by=2".to_string();
    if let Some(fps) = fps {
        filters.push_str(&format!(",fps={fps}"));
    }
    let mut arguments: Vec<OsString> = [
        "-hide_banner",
        "-loglevel",
        "error",
        "-nostdin",
        "-y",
        "-protocol_whitelist",
        "file",
        "-ignore_loop",
        "1",
        "-i",
    ]
    .into_iter()
    .map(Into::into)
    .collect();
    if source.extension().and_then(OsStr::to_str) != Some("gif") {
        arguments.drain(7..9);
    }
    arguments.push(source.into());
    arguments.extend(
        [
            "-map",
            "0:V:0",
            "-an",
            "-sn",
            "-dn",
            "-vf",
            &filters,
            "-c:v",
            "libx264",
            "-preset",
            "veryfast",
            "-crf",
            "20",
            "-pix_fmt",
            "yuv420p",
            "-movflags",
            "+faststart",
            "-f",
            "mp4",
        ]
        .into_iter()
        .map(OsString::from),
    );
    arguments.push(path.into());
    output(&tools.ffmpeg, arguments)?;
    inspect(tools, path, "mp4")?;
    Ok(())
}

fn inspect(tools: &Tools, source: &Path, extension: &str) -> Result<()> {
    let data = output(
        &tools.ffprobe,
        [
            OsStr::new("-v"),
            OsStr::new("error"),
            OsStr::new("-protocol_whitelist"),
            OsStr::new("file"),
            OsStr::new("-select_streams"),
            OsStr::new("V:0"),
            OsStr::new("-show_entries"),
            OsStr::new("stream=width,height:format=format_name"),
            OsStr::new("-of"),
            OsStr::new("json"),
            source.as_os_str(),
        ],
    )?;
    let info: serde_json::Value =
        serde_json::from_slice(&data.stdout).map_err(|e| e.to_string())?;
    let format = info["format"]["format_name"].as_str().unwrap_or_default();
    let stream = &info["streams"][0];
    if !format.split(',').any(|part| part == extension)
        || stream["width"].as_u64().unwrap_or_default() < 2
        || stream["height"].as_u64().unwrap_or_default() < 2
    {
        return Err("Die Datei enthält kein gültiges Video im gewählten Format.".into());
    }
    Ok(())
}

pub fn import(
    tools: &Tools,
    root: &Path,
    source: &Path,
    fps: Option<u8>,
) -> Result<(Wallpaper, TempDir)> {
    let extension = source
        .extension()
        .and_then(OsStr::to_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !matches!(extension.as_str(), "mp4" | "webm" | "gif") || !source.is_file() {
        return Err("Bitte eine MP4-, WebM- oder GIF-Datei auswählen.".into());
    }
    inspect(tools, source, &extension)?;
    let staging = tempfile::Builder::new()
        .prefix(".junadesk-import-")
        .tempdir_in(root)
        .map_err(|e| e.to_string())?;
    let directory = staging.path();
    let original = directory.join(format!("original.{extension}"));
    fs::copy(source, &original).map_err(|e| e.to_string())?;
    let video = rendition(directory, None);
    encode(tools, &original, &video, None)?;
    if fps.is_some() {
        encode(tools, &original, &rendition(directory, fps), fps)?;
    }
    output(
        &tools.ffmpeg,
        [
            OsStr::new("-hide_banner"),
            OsStr::new("-loglevel"),
            OsStr::new("error"),
            OsStr::new("-nostdin"),
            OsStr::new("-i"),
            video.as_os_str(),
            OsStr::new("-frames:v"),
            OsStr::new("1"),
            OsStr::new("-vf"),
            OsStr::new(
                "scale=w='min(iw,640)':h='min(ih,360)':force_original_aspect_ratio=decrease",
            ),
            directory.join("preview.jpg").as_os_str(),
        ],
    )?;
    let name = source
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .trim()
        .chars()
        .take(100)
        .collect::<String>();
    Ok((
        Wallpaper {
            id: Uuid::new_v4().to_string(),
            name: if name.is_empty() {
                "Wallpaper".into()
            } else {
                name
            },
            extension,
        },
        staging,
    ))
}

pub fn prepare_limit(
    tools: &Tools,
    directory: &Path,
    wallpaper: &Wallpaper,
    fps: Option<u8>,
) -> Result<()> {
    let destination = rendition(directory, fps);
    if destination.is_file() {
        return Ok(());
    }
    let source = directory.join(format!("original.{}", wallpaper.extension));
    let file = NamedTempFile::new_in(directory).map_err(|e| e.to_string())?;
    let path = file.into_temp_path();
    encode(tools, &source, &path, fps)?;
    path.persist(destination).map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires FFmpeg with libx264 and FFprobe"]
    fn imports_supported_media_and_caches_limited_video() {
        let resources = Path::new(env!("CARGO_MANIFEST_DIR")).join("resources");
        let tools = Tools {
            ffmpeg: binary(&resources, "ffmpeg"),
            ffprobe: binary(&resources, "ffprobe"),
        };
        let workspace = tempfile::tempdir().unwrap();
        let root = workspace.path();
        for (extension, codec) in [("mp4", "libx264"), ("webm", "libvpx-vp9"), ("gif", "gif")] {
            let source = root.join(format!("Test & Übung.{extension}"));
            let mut arguments = vec![
                OsString::from("-v"),
                "error".into(),
                "-f".into(),
                "lavfi".into(),
                "-i".into(),
                "testsrc2=size=96x64:rate=30:duration=1".into(),
                "-c:v".into(),
                codec.into(),
            ];
            arguments.push(source.as_os_str().into());
            // Release tools include VP9 decoding. The fixture avoids requiring a VP9 encoder.
            let result = output(&tools.ffmpeg, arguments);
            if extension == "webm" && result.is_err() {
                let fixture = include_bytes!("../testdata/test-vp9.webm");
                fs::write(&source, fixture).unwrap();
            } else {
                result.unwrap();
            }
            let (wallpaper, staging) = import(&tools, root, &source, Some(30)).unwrap();
            assert_eq!(wallpaper.extension, extension);
            assert_eq!(wallpaper.name, "Test & Übung");
            assert!(staging.path().join("preview.jpg").is_file());
            assert!(staging
                .path()
                .join(format!("original.{extension}"))
                .is_file());
            assert!(rendition(staging.path(), Some(30)).is_file());
            prepare_limit(&tools, staging.path(), &wallpaper, Some(15)).unwrap();
            let limited = rendition(staging.path(), Some(15));
            let data = output(
                &tools.ffprobe,
                [
                    OsStr::new("-v"),
                    OsStr::new("error"),
                    OsStr::new("-select_streams"),
                    OsStr::new("v:0"),
                    OsStr::new("-show_entries"),
                    OsStr::new("stream=avg_frame_rate"),
                    OsStr::new("-of"),
                    OsStr::new("csv=p=0"),
                    limited.as_os_str(),
                ],
            )
            .unwrap();
            assert_eq!(String::from_utf8_lossy(&data.stdout).trim(), "15/1");
        }
        let invalid = root.join("defekt.mp4");
        fs::write(&invalid, b"invalid").unwrap();
        assert!(import(&tools, root, &invalid, None).is_err());
    }
}
