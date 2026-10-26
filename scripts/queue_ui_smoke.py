"""Capture the native queue UI with an isolated, non-networked sample queue."""
import argparse
import json
import time
import os
from pathlib import Path
import sqlite3
import subprocess
import uuid
import struct
import zlib

parser = argparse.ArgumentParser()
parser.add_argument("--retry", action="store_true", help="Show a future retry countdown with selected-file details")
parser.add_argument("--quota-reached", action="store_true", help="Preview an exhausted hourly data quota")
parser.add_argument("--bandwidth", action="store_true", help="Preview saved speed limits and the selected-file dialog")
parser.add_argument("--preferences", action="store_true", help="Preview the tray and notifications preferences")
parser.add_argument("--schedule", action="store_true", help="Preview recurring schedule settings")
parser.add_argument("--synchronize", action="store_true", help="Preview synchronization results and queue controls")
parser.add_argument("--folders", action="store_true", help="Preview category folder settings")
parser.add_argument("--width", type=int, default=1280)
parser.add_argument("--height", type=int, default=860)
parser.add_argument("--dark", action="store_true")
parser.add_argument("--compact", action="store_true")
parser.add_argument("--hide-details", action="store_true")
parser.add_argument("--custom-columns", action="store_true", help="Preview a saved column order and custom widths")
parser.add_argument("--view", choices=["downloads", "queues", "scheduler", "settings", "browsers", "help"], default="downloads")
parser.add_argument("--feature", choices=["columns", "checksum", "drop", "add", "completion", "scan-settings", "scan-result", "grabber", "grabber-setup", "grabber-project", "grabber-schedule", "grabber-discoveries", "grabber-scheduler", "quotas"])
args = parser.parse_args()
if not 860 <= args.width <= 1920 or not 600 <= args.height <= 1200:
    parser.error("Unsupported fixture size")
if sum((args.retry, args.bandwidth, args.preferences, args.folders, args.schedule)) > 1:
    parser.error("Choose one preview")
root = Path(__file__).resolve().parents[1]
run_root = root / "artifacts" / ("queue-ui-" + uuid.uuid4().hex[:8])
data = run_root / "data"
downloads = run_root / "downloads"
data.mkdir(parents=True)
downloads.mkdir()
(data / "parts").mkdir()
completed = downloads / "Product-guide.txt"
completed.write_text("Download Engine layout verification.\n", encoding="utf-8")

names = [
    ("Studio-installer.exe", "paused", 510, 327, True),
    ("Design-assets.zip", "queued", 2048, 820, True),
    ("Reference-photos.zip", "paused", 400, 156, False),
    ("Product-guide.txt", "complete", 0, 0, False),
    ("Material-pack.zip", "failed", 760, 84, False),
]
if args.retry:
    # One sample with a future deadline: no network work is runnable during capture.
    names = [("Sample-archive.zip", "queued", 1200, 600, True)]
if args.synchronize:
    names = [("Catalog.zip", "complete", 0, 0, False),
             ("Release-notes.txt", "complete", 0, 0, False),
             ("Protected-archive.bin", "failed", 0, 0, False)]
with sqlite3.connect(data / "queue.sqlite3") as conn:
    conn.execute("CREATE TABLE jobs (id TEXT PRIMARY KEY, request_id TEXT NOT NULL UNIQUE, created INTEGER NOT NULL, data TEXT NOT NULL)")
    for index, (name, status, total, done, resumable) in enumerate(names):
        total *= 1024 * 1024
        done *= 1024 * 1024
        if status == "complete":
            total = done = completed.stat().st_size
        job_id = str(uuid.uuid4())
        url = "https://example.com/" + name
        plan = None
        if resumable:
            plan = {"size": total, "etag": '"fixture-v1"', "final_url": url,
                    "segments": [{"start": n * (total // 4), "end": (n + 1) * (total // 4) - 1} for n in range(4)]}
        job = {
            "id": job_id, "request_id": job_id, "name": name, "url": url,
            "directory": str(downloads), "status": status, "total": total,
            "downloaded": done, "plan": plan,
            "saved_path": str(completed) if status == "complete" else None,
            "error": "Server returned HTTP 403. The link may have expired." if status == "failed" else None,
            "created": int(time.time()) + index,
        }
        if args.retry:
            job["retry"] = {"attempts": 2, "next_at": time.time_ns() // 1_000_000 + 120000}
            job["error"] = "Server returned HTTP 503."
        if args.synchronize:
            job["synchronization"] = {"phase": ["updated", "up_to_date", "checking"][index],
                "checked_at": int(time.time()), "previous": {"path": str(downloads / ("previous-" + name)),
                "downloaded": done, "total": total, "plan": None, "sha256": None,
                "remote": None, "expected_mime": None}}
        conn.execute("INSERT INTO jobs VALUES (?,?,?,?)", (job_id, job_id, job["created"], json.dumps(job)))

# Keep all sample files offline; the native app migrates their memberships.
with sqlite3.connect(data / "queue.sqlite3") as conn:
    conn.execute("CREATE TABLE download_queues(id TEXT PRIMARY KEY,name TEXT NOT NULL UNIQUE COLLATE NOCASE,max_active INTEGER NOT NULL CHECK(max_active BETWEEN 1 AND 8),running INTEGER NOT NULL,scheduled_at INTEGER)")
    conn.execute("INSERT INTO download_queues VALUES('main','Main queue',3,?,NULL)", (int(args.retry),))
    conn.execute("INSERT INTO download_queues VALUES('later','Later',2,0,NULL)")
    if args.synchronize:
        conn.execute("UPDATE download_queues SET name='Current files',running=0 WHERE id='main'")
        conn.execute("CREATE TABLE synchronization_queues(queue_id TEXT PRIMARY KEY REFERENCES download_queues(id) ON DELETE CASCADE,last_window_start INTEGER)")
        conn.execute("INSERT INTO synchronization_queues(queue_id) VALUES('main')")
    if args.view == "scheduler":
        # Main is suspended; the only enabled sample queue is empty, so every
        # preview remains offline regardless of the machine's local clock.
        conn.execute("CREATE TABLE queue_schedules(queue_id TEXT PRIMARY KEY REFERENCES download_queues(id) ON DELETE CASCADE, days INTEGER NOT NULL CHECK(days BETWEEN 1 AND 127), start_minute INTEGER NOT NULL CHECK(start_minute BETWEEN 0 AND 1439), stop_minute INTEGER NOT NULL CHECK(stop_minute BETWEEN 0 AND 1439 AND stop_minute<>start_minute), enabled INTEGER NOT NULL CHECK(enabled IN (0,1)))")
        conn.execute("INSERT INTO queue_schedules VALUES('main',21,1380,420,0)")
        conn.execute("INSERT INTO queue_schedules VALUES('later',127,0,420,1)")

if args.bandwidth:
    with sqlite3.connect(data / "queue.sqlite3") as conn:
        conn.execute("CREATE TABLE bandwidth_settings(id INTEGER PRIMARY KEY CHECK(id=1),bytes_per_second INTEGER CHECK(bytes_per_second BETWEEN 1024 AND 1073741824))")
        conn.execute("INSERT INTO bandwidth_settings VALUES(1,2097152)")
        conn.execute("CREATE TABLE bandwidth_downloads(job_id TEXT PRIMARY KEY REFERENCES jobs(id) ON DELETE CASCADE,bytes_per_second INTEGER NOT NULL CHECK(bytes_per_second BETWEEN 1024 AND 1073741824))")
        conn.execute("INSERT INTO bandwidth_downloads SELECT id,262144 FROM jobs WHERE json_extract(data,'$.status') != 'complete'")
with sqlite3.connect(data / "queue.sqlite3") as conn:
    conn.execute("CREATE TABLE interface_preferences(id INTEGER PRIMARY KEY CHECK(id=1),data TEXT NOT NULL)")
    preferences = dict(appearance="dark" if args.dark else "light", compact_rows=args.compact,
                       show_details=not args.hide_details, sidebar_labels=True)
    if args.custom_columns:
        preferences["table_layout"] = dict(
            order=["number","name","size","status","progress","speed","eta","added","actions"],
            widths=[None, 420, None, None, 120, None, None, None, None])
    conn.execute("INSERT INTO interface_preferences VALUES(1,?)", (json.dumps(preferences),))
if args.quota_reached:
    with sqlite3.connect(data / "queue.sqlite3") as conn:
        conn.execute("CREATE TABLE data_quotas(id INTEGER PRIMARY KEY,hourly_limit INTEGER,daily_limit INTEGER,hourly_used INTEGER NOT NULL,daily_used INTEGER NOT NULL,hour_start INTEGER,day_start INTEGER)")
        conn.execute("INSERT INTO data_quotas VALUES(1,1073741824,4294967296,1073741824,2147483648,?,?)", (int(time.time()),int(time.time())))
screenshot = run_root / "native-ui.ppm"
env = dict(os.environ, IDM_EX_HOME=str(run_root))
env.pop("EFRAME_SCREENSHOT_TO", None)
env.pop("IDM_EX_SCREENSHOT_TO", None)
env.pop("IDM_EX_TEST_SPEED_DIALOG", None)
env["IDM_EX_SCREENSHOT_TO"] = str(screenshot)
env["IDM_EX_TEST_SIZE"] = f"{args.width}x{args.height}"
env["IDM_EX_TEST_VIEW"] = args.view
env.pop("IDM_EX_TEST_PREFERENCES", None)
if args.bandwidth: env["IDM_EX_TEST_SPEED_DIALOG"] = "1"
if args.preferences: env["IDM_EX_TEST_PREFERENCES"] = "1"
env.pop("IDM_EX_TEST_FOLDERS", None)
if args.folders: env["IDM_EX_TEST_FOLDERS"] = "1"
env.pop("IDM_EX_TEST_SCHEDULE", None)
if args.schedule: env["IDM_EX_TEST_SCHEDULE"] = "1"
env.pop("IDM_EX_TEST_FEATURE", None)
if args.feature: env["IDM_EX_TEST_FEATURE"] = args.feature
startup = subprocess.STARTUPINFO()
startup.dwFlags = subprocess.STARTF_USESHOWWINDOW
startup.wShowWindow = 0
result = subprocess.run([str(root / "target" / "debug" / "de.exe")],
                        env=env, startupinfo=startup, capture_output=True, timeout=30)
assert result.returncode == 0, result.stderr.decode(errors="replace")
assert screenshot.exists() and screenshot.stat().st_size > 0
if args.retry:
    with sqlite3.connect(data / "queue.sqlite3") as conn:
        saved = json.loads(conn.execute("SELECT data FROM jobs").fetchone()[0])
    assert saved["status"] == "queued" and saved["retry"]["attempts"] == 2
    assert saved["retry"]["next_at"] > time.time_ns() // 1_000_000
    assert saved["downloaded"] == 600 * 1024 * 1024
# PPM is emitted directly by egui; convert losslessly without an image dependency.
raw = screenshot.read_bytes()
magic, dimensions, maximum, pixels = raw.split(b"\n", 3)
assert magic == b"P6" and maximum == b"255"
width, height = map(int, dimensions.split())
assert len(pixels) == width * height * 3


def chunk(kind, data):
    return (struct.pack(">I", len(data)) + kind + data
            + struct.pack(">I", zlib.crc32(kind + data) & 0xffffffff))


scanlines = b"".join(b"\0" + pixels[y*width*3:(y+1)*width*3] for y in range(height))
png = (b"\x89PNG\r\n\x1a\n"
       + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
       + chunk(b"IDAT", zlib.compress(scanlines)) + chunk(b"IEND", b""))
screenshot = screenshot.with_suffix(".png")
screenshot.write_bytes(png)
print(json.dumps({"screenshot": str(screenshot), "data_root": str(run_root), "network_jobs": 0,
                  "width": width, "height": height, "dark": args.dark, "compact": args.compact,
                  "view": args.view, "feature": args.feature, "custom_columns": args.custom_columns, "retry_preview": args.retry, "bandwidth_preview": args.bandwidth,
                  "preferences_preview": args.preferences, "folders_preview": args.folders, "schedule_preview": args.schedule,
                  "synchronization_preview": args.synchronize}))
