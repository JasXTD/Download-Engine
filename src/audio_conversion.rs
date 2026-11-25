//! Convert the first audio track of a local file to MP3 or PCM WAV.
use crate::{child_process::Running, download_file, file_relocation, hls::merge::Helper};
use anyhow::{Context, Result, bail, ensure};
use std::{
    fs::{self, File},
    io::{self, Read, Seek, Write},
    path::{Path, PathBuf},
    process::Stdio,
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

// Only self-contained file formats. In particular, exclude manifests, concat,
// devices and any demuxer that opens other resources by design.
const FORMATS: &str = "mov,mp4,m4a,3gp,3g2,mj2,matroska,webm,ogg,wav,flac,mp3,aac,ac3,eac3,aiff,ape,asf,avi,flv,mpeg,mpegts,au,wv,tta,amr,caf";
const MAX_OUTPUT: u64 = 4 * 1024 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u16)]
pub enum Mp3Bitrate {
    Kbps128 = 128,
    #[default]
    Kbps192 = 192,
    Kbps256 = 256,
    Kbps320 = 320,
}

impl Mp3Bitrate {
    pub const ALL: [Self; 4] = [Self::Kbps128, Self::Kbps192, Self::Kbps256, Self::Kbps320];

    pub fn kbps(self) -> u16 {
        self as u16
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Format {
    #[default]
    Mp3,
    Wav,
}

impl Format {
    pub const ALL: [Self; 2] = [Self::Mp3, Self::Wav];

    pub fn label(self) -> &'static str {
        match self {
            Self::Mp3 => "MP3",
            Self::Wav => "WAV",
        }
    }
    pub fn extension(self) -> &'static str {
        match self {
            Self::Mp3 => "mp3",
            Self::Wav => "wav",
        }
    }
    pub fn matches(self, path: &Path) -> bool {
        path.extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case(self.extension()))
    }
    pub fn suggested_output(self, source: &Path) -> PathBuf {
        if self.matches(source) {
            let mut stem = source.file_stem().unwrap_or_default().to_owned();
            stem.push("-converted.");
            stem.push(self.extension());
            source.with_file_name(stem)
        } else {
            source.with_extension(self.extension())
        }
    }
}

struct Partial {
    file: File,
    path: PathBuf,
    remove: bool,
}

impl Drop for Partial {
    fn drop(&mut self) {
        if self.remove {
            let _ = file_relocation::delete_owned(&self.file, &self.path);
        }
    }
}

/// Runs on a worker thread. The final pathname appears only after both helpers
/// exit successfully. An existing file, including the source, is never replaced.
pub fn convert(source: &Path, destination: &Path, cancel: &CancellationToken) -> Result<()> {
    convert_with_bitrate(source, destination, Mp3Bitrate::default(), cancel)
}

pub fn convert_with_bitrate(
    source: &Path,
    destination: &Path,
    bitrate: Mp3Bitrate,
    cancel: &CancellationToken,
) -> Result<()> {
    ensure!(!cancel.is_cancelled(), "Conversion cancelled");
    let format = Format::ALL
        .into_iter()
        .find(|format| format.matches(destination))
        .context("Choose an output filename ending in .mp3 or .wav")?;
    let _source_dirs = download_file::guard_parents(source)?;
    let destination_dirs = download_file::guard_parents(destination)?;
    let input =
        download_file::read_completed(source).context("Cannot read the selected media file")?;
    match fs::symlink_metadata(destination) {
        Ok(_) => bail!("The output file already exists. Choose another name."),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e).context("Cannot check the output location"),
    }
    let decoder = Helper::ffmpeg(cancel)?;
    let path = destination.with_file_name(format!(".de-convert-{}.part", Uuid::new_v4()));
    let mut partial = Partial {
        file: file_relocation::open_owned(&path, true)?,
        path,
        remove: true,
    };
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::{
            ERROR_INVALID_FUNCTION, ERROR_INVALID_NAME, ERROR_NOT_SUPPORTED,
        };
        let mut stream = source.as_os_str().to_owned();
        stream.push(":Zone.Identifier");
        match fs::metadata(&stream) {
            Ok(_) => download_file::mark_internet(&partial.path)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            // Volumes such as FAT cannot store alternate data streams.
            Err(e)
                if matches!(
                    e.raw_os_error().map(|n| n as u32),
                    Some(ERROR_INVALID_FUNCTION | ERROR_INVALID_NAME | ERROR_NOT_SUPPORTED)
                ) => {}
            Err(e) => return Err(e).context("Cannot check the source file's Internet zone"),
        }
    }
    let mut decode = Helper::spawn(
        decoder
            .command()
            .env_remove("FFREPORT")
            .args([
                "-nostdin",
                "-hide_banner",
                "-loglevel",
                "error",
                "-xerror",
                "-abort_on",
                "empty_output+empty_output_stream",
                "-max_alloc",
                "268435456",
                "-protocol_whitelist",
                "fd",
                "-format_whitelist",
                FORMATS,
                "-threads",
                "1",
                "-i",
                "fd:",
                "-map",
                "0:a:0",
                "-ac",
                "2",
                "-ar",
                "44100",
                "-c:a",
                "pcm_s16le",
                "-f",
                "s16le",
                "pipe:1",
            ])
            .stdin(input)
            .stdout(Stdio::piped())
            .stderr(Stdio::null()),
        cancel,
        Some(1024 * 1024 * 1024),
    )?;
    let pcm = decode
        .child
        .stdout
        .take()
        .context("Cannot open the decoder's audio pipe")?;
    let (encoded, encode) = match format {
        Format::Wav => (pcm, None),
        Format::Mp3 => {
            let encoder = Helper::mp3_encoder(cancel)?;
            let mut encode = Helper::spawn(
                encoder
                    .command()
                    .env_remove("LAMEOPT")
                    .args([
                        "--silent",
                        "--noreplaygain",
                        "-r",
                        "-s",
                        "44.1",
                        "--bitwidth",
                        "16",
                        "--signed",
                        "--little-endian",
                        "-m",
                        "j",
                        "-b",
                        &bitrate.kbps().to_string(),
                        "-",
                        "-",
                    ])
                    .stdin(pcm)
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null()),
                cancel,
                Some(256 * 1024 * 1024),
            )?;
            let stream = encode
                .child
                .stdout
                .take()
                .context("Cannot open the encoder's MP3 pipe")?;
            (stream, Some(encode))
        }
    };
    let stop = cancel.child_token();
    let written = std::thread::scope(|scope| -> Result<u64> {
        let watch = std::thread::Builder::new()
            .name("audio-converter".into())
            .spawn_scoped(scope, || wait_for_helpers(decode, encode, &stop))
            .context("Cannot monitor the media helpers")?;
        // The encoder can exit successfully after its final buffered write fails.
        // Write the bytes ourselves and publish only after every write succeeds.
        let copied = match format {
            Format::Mp3 => write_audio(encoded, &partial.file, MAX_OUTPUT),
            Format::Wav => write_wav(encoded, &mut partial.file),
        };
        if copied.is_err() {
            stop.cancel();
        }
        let finished = watch
            .join()
            .map_err(|_| anyhow::anyhow!("Media helper monitor stopped"))?;
        let written = copied?;
        finished?;
        Ok(written)
    })?;
    ensure!(written > 0, "The converter returned no audio");
    partial
        .file
        .sync_all()
        .context("Cannot finish writing the audio file. Check free space in the output folder.")?;
    ensure!(!cancel.is_cancelled(), "Conversion cancelled");
    #[cfg(windows)]
    {
        let _guards = destination_dirs;
        file_relocation::rename_owned(&partial.file, &partial.path, destination)?;
    }
    #[cfg(unix)]
    {
        let mut claim = crate::unix_files::VerifiedLink::new(
            &partial.file,
            &partial.path,
            partial.path.with_extension("claim"),
        )?;
        claim
            .publish(
                destination_dirs
                    .last()
                    .context("Missing destination directory")?,
                destination.file_name().context("Missing output filename")?,
            )
            .context("Cannot save the audio file without replacing an existing file")?;
    }
    // Unix published a verified hard link. Windows renamed the open file itself.
    partial.remove = cfg!(unix);
    Ok(())
}

fn write_audio(mut encoded: impl Read, mut output: impl Write, limit: u64) -> Result<u64> {
    let written = io::copy(&mut encoded.by_ref().take(limit), &mut output)
        .context("Cannot write the audio file. Check free space in the output folder.")?;
    if written == limit {
        ensure!(
            io::copy(&mut encoded.take(1), &mut io::sink())? == 0,
            "Audio exceeds the output size limit"
        );
    }
    Ok(written)
}

fn write_wav(pcm: impl Read, file: &mut File) -> Result<u64> {
    // FFmpeg sends stereo 44.1 kHz, signed 16-bit little-endian PCM down the pipe.
    // Fill in the RIFF lengths using the guarded output handle after the last sample.
    let mut header = *b"RIFF\0\0\0\0WAVEfmt \x10\0\0\0\x01\0\x02\0\x44\xac\0\0\x10\xb1\x02\0\x04\0\x10\0data\0\0\0\0";
    file.write_all(&header)
        .context("Cannot write the WAV header")?;
    let bytes = write_audio(pcm, &mut *file, MAX_OUTPUT - header.len() as u64)?;
    ensure!(bytes % 4 == 0, "Incomplete PCM sample frame");
    header[4..8].copy_from_slice(&(bytes as u32 + 36).to_le_bytes());
    header[40..44].copy_from_slice(&(bytes as u32).to_le_bytes());
    file.rewind()?;
    file.write_all(&header)
        .context("Cannot finish the WAV header")?;
    Ok(bytes)
}

fn wait_for_helpers(
    mut decode: Running,
    mut encode: Option<Running>,
    cancel: &CancellationToken,
) -> Result<()> {
    let started = Instant::now();
    let mut decoded = false;
    while !decoded || encode.is_some() {
        ensure!(!cancel.is_cancelled(), "Conversion cancelled");
        ensure!(
            started.elapsed() < Duration::from_secs(8 * 3600),
            "Conversion exceeded the time limit"
        );
        if !decoded && let Some(status) = decode.try_wait()? {
            ensure!(
                status.success(),
                "Cannot decode audio. Choose a supported local media file with an audio track."
            );
            decoded = true;
        }
        if let Some(encoder) = &mut encode
            && let Some(status) = encoder.try_wait()?
        {
            ensure!(status.success(), "MP3 encoding failed.");
            encode = None;
        }
        if !decoded || encode.is_some() {
            std::thread::sleep(Duration::from_millis(40));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
