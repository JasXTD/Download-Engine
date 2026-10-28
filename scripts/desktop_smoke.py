"""Exercise the actual desktop and native host against an isolated local server."""
import argparse
import ctypes
from ctypes import wintypes
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
from profile_database import open_profile_database
import struct
import subprocess
import threading
import time
import uuid

def main():
    parser=argparse.ArgumentParser()
    parser.add_argument("--binary-dir",default="target/debug")
    parser.add_argument("--capture",action="store_true")
    parser.add_argument("--automatic",action="store_true",help="Exercise prepared/committed automatic handoff")
    parser.add_argument("--retry",action="store_true",help="Fail the first actual download request with HTTP 503")
    parser.add_argument("--bandwidth",action="store_true",help="Verify persistent shared 2 MiB/s cap on all four ranges")
    parser.add_argument("--quotas",action="store_true",help="Verify a quota hold and resume after a controlled hourly reset")
    parser.add_argument("--folders",action="store_true",help="Verify a custom Documents destination shared with the native host")
    parser.add_argument("--adaptive",action="store_true",help="Verify a 32 MiB uneven transfer rebalances ranges in the desktop")
    parser.add_argument("--synchronize",action="store_true",help="Check changed, unchanged, and failed updates in an isolated synchronization queue")
    av=parser.add_mutually_exclusive_group()
    av.add_argument("--antivirus-scanner",help="Use an isolated command-line scanner fixture .exe")
    av.add_argument("--antivirus-defender",action="store_true",help="Run installed Defender only against the harmless downloaded fixture")
    av.add_argument("--antivirus-malwarebytes",action="store_true",help="Request an installed Malwarebytes scan of the harmless fixture; review its own report")
    parser.add_argument("--antivirus-mode",choices=["clean","detected","error","timeout"],default="clean")
    parser.add_argument("--antivirus-expected",default="no_threats_found",help="Expected recorded antivirus verdict")
    args=parser.parse_args()
    has_antivirus=bool(args.antivirus_scanner or args.antivirus_defender or args.antivirus_malwarebytes)
    if args.adaptive and args.bandwidth:
        parser.error("--adaptive measures uneven connections without an artificial speed cap")
    if args.quotas and (args.adaptive or args.synchronize or args.retry):
        parser.error("--quotas uses an ordinary resumable transfer")
    if args.retry and args.automatic:
        parser.error("--retry uses manual handoff; automatic capture must pass anonymous preflight")
    if args.synchronize and (args.adaptive or args.retry or args.bandwidth or args.antivirus_defender or args.antivirus_malwarebytes):
        parser.error("--synchronize uses ordinary transfers and optional --antivirus-scanner fixtures only")
    root=Path(__file__).resolve().parents[1]
    binaries=(root/args.binary_dir).resolve()
    run_root=root/"artifacts"/(("desktop scan & smoke-" if has_antivirus else "desktop-smoke-")+uuid.uuid4().hex[:8])
    run_root.mkdir(parents=True)
    payload=bytes(range(251))*40000
    if args.adaptive:
        payload=(bytes(range(251))*(32*1024*1024//251+1))[:32*1024*1024]
    requests=[]
    payload_started=[]
    request_lock=threading.Lock()
    version='"desktop-smoke-v1"'
    sync_failure=False
    class Handler(BaseHTTPRequestHandler):
        def log_message(self,*args): pass
        def do_GET(self):
            with request_lock:
                first=not requests
                requests.append(time.time_ns()//1_000_000)
            if sync_failure or self.headers.get("If-None-Match")==version:
                self.send_response(404 if sync_failure else 304)
                self.send_header("Content-Length","0")
                self.send_header("ETag",version)
                self.end_headers()
                return
            if args.retry and first:
                self.send_response(503)
                self.send_header("Content-Length","0")
                self.send_header("Connection","close")
                self.end_headers()
                return
            start,end=0,len(payload)-1
            range_value=self.headers.get("Range")
            if range_value:
                start,end=map(int,range_value.removeprefix("bytes=").split("-"))
            if range_value != "bytes=0-0":
                with request_lock: payload_started.append(time.monotonic())
            self.send_response(206 if range_value else 200)
            self.send_header("Content-Length",str(end-start+1))
            self.send_header("ETag",version)
            self.send_header("Content-Type","application/octet-stream")
            if range_value:
                self.send_header("Content-Range",f"bytes {start}-{end}/{len(payload)}")
            self.end_headers()
            try:
                if args.quotas and range_value != "bytes=0-0":
                    for offset in range(start,end+1,32768):
                        self.wfile.write(payload[offset:min(offset+32768,end+1)])
                        time.sleep(.005)
                elif args.adaptive and range_value != "bytes=0-0":
                    delay=.05 if start==0 and end==len(payload)//4-1 else .003
                    for offset in range(start,end+1,128*1024):
                        time.sleep(delay)
                        self.wfile.write(payload[offset:min(offset+128*1024,end+1)])
                else:
                    self.wfile.write(payload[start:end+1])
            except (ConnectionResetError,BrokenPipeError,ConnectionAbortedError): pass
    server=ThreadingHTTPServer(("127.0.0.1",0),Handler)
    threading.Thread(target=server.serve_forever,daemon=True).start()
    env=dict(os.environ,IDM_EX_HOME=str(run_root))
    env.pop("EFRAME_SCREENSHOT_TO",None)
    startup=subprocess.STARTUPINFO()
    startup.dwFlags=subprocess.STARTF_USESHOWWINDOW
    startup.wShowWindow=0
    stderr=(run_root/"desktop.log").open("w",encoding="utf-8")
    initial_data=run_root/"data"
    initial_data.mkdir()
    with open_profile_database(initial_data/"queue.sqlite3") as conn:
        # Unrelated transfer fixtures should not place notifications on the desktop.
        conn.execute("CREATE TABLE desktop_preferences(id INTEGER PRIMARY KEY CHECK(id=1),close_to_tray INTEGER NOT NULL,notify_complete INTEGER NOT NULL,notify_failed INTEGER NOT NULL)")
        conn.execute("INSERT INTO desktop_preferences VALUES(1,0,0,0)")
    if args.bandwidth:
        with open_profile_database(initial_data/"queue.sqlite3") as conn:
            conn.execute("CREATE TABLE bandwidth_settings(id INTEGER PRIMARY KEY CHECK(id=1),bytes_per_second INTEGER CHECK(bytes_per_second BETWEEN 1024 AND 1073741824))")
            conn.execute("INSERT INTO bandwidth_settings VALUES(1,2097152)")
    if args.quotas:
        with open_profile_database(initial_data/"queue.sqlite3") as conn:
            conn.execute("CREATE TABLE data_quotas(id INTEGER PRIMARY KEY,hourly_limit INTEGER,daily_limit INTEGER,hourly_used INTEGER NOT NULL,daily_used INTEGER NOT NULL,hour_start INTEGER,day_start INTEGER)")
            conn.execute("INSERT INTO data_quotas VALUES(1,1048576,33554432,0,0,?,?)",(int(time.time()),int(time.time())))
    if has_antivirus:
        scan_settings={"enabled":True,"provider":"malwarebytes" if args.antivirus_malwarebytes else "defender" if args.antivirus_defender else "custom","timeout_seconds":10}
        if args.antivirus_scanner:
            scan_settings.update(executable=str(Path(args.antivirus_scanner).resolve()),arguments=[args.antivirus_mode,"{file}"],clean_exit=0,detected_exit=1,custom_confirmed=True)
        with open_profile_database(initial_data/"queue.sqlite3") as conn:
            conn.execute("CREATE TABLE antivirus_settings(id INTEGER PRIMARY KEY CHECK(id=1),data TEXT NOT NULL)")
            conn.execute("INSERT INTO antivirus_settings VALUES(1,?)",(json.dumps(scan_settings),))
    expected_folder=run_root/"downloads"/"Other"
    filename="desktop-test.bin"
    if args.folders:
        expected_folder=run_root/"custom documents"
        filename="desktop-test.pdf"
        with open_profile_database(initial_data/"queue.sqlite3") as conn:
            conn.execute("CREATE TABLE download_folders(id INTEGER PRIMARY KEY CHECK(id=1),data TEXT NOT NULL)")
            conn.execute("INSERT INTO download_folders VALUES(1,?)",(json.dumps({"organize":True,"overrides":{"documents":str(expected_folder)}}),))
    desktop=subprocess.Popen([str(binaries/"de.exe")],env=env,stdin=subprocess.DEVNULL,stdout=stderr,stderr=stderr,startupinfo=startup)
    try:
        database=run_root/"data"/"queue.sqlite3"
        deadline=time.monotonic()+25
        def database_ready():
            if not database.exists(): return False
            with open_profile_database(database) as conn:
                return conn.execute("SELECT count(*) FROM sqlite_master WHERE type='table' AND name='jobs'").fetchone()[0] == 1
        while not database_ready():
            if desktop.poll() is not None: raise RuntimeError("Desktop exited before opening its queue")
            if time.monotonic()>deadline: raise TimeoutError("Desktop queue did not open")
            time.sleep(.05)
        request={"action":"add","request_id":str(uuid.uuid4()),"url":f"http://127.0.0.1:{server.server_port}/{filename}"}
        if args.automatic:
            capture_id="capture:"+str(uuid.uuid4())
            prepared=dict(action="capture_prepare",request_id=capture_id,url=request["url"],
                filename=filename,size=len(payload),mime="application/octet-stream")
            prepare_data=json.dumps(prepared).encode()
            prepare_host=subprocess.run([str(binaries/"de-host.exe")],
                input=struct.pack("<I",len(prepare_data))+prepare_data,env=env,capture_output=True,timeout=15)
            assert prepare_host.returncode==0,prepare_host.stderr.decode()
            prepare_reply=json.loads(prepare_host.stdout[4:])
            assert prepare_reply["ok"] and prepare_reply["capture_state"]=="prepared",prepare_reply
            with open_profile_database(database) as conn:
                assert conn.execute("SELECT count(*) FROM jobs").fetchone()[0]==0
            request=dict(action="capture_commit",request_id=capture_id)
        data=json.dumps(request).encode()
        transfer_started=time.monotonic()
        host=subprocess.run([str(binaries/"de-host.exe")],input=struct.pack("<I",len(data))+data,env=env,capture_output=True,timeout=15)
        assert host.returncode==0,host.stderr.decode()
        length=struct.unpack("<I",host.stdout[:4])[0]
        assert len(host.stdout)==4+length,"Unexpected stdout outside protocol"
        reply=json.loads(host.stdout[4:])
        assert reply["ok"] and reply["message"]=="Added to the download queue",reply
        job=None
        retry_state=None
        quota_hold=None
        while time.monotonic()<deadline:
            with open_profile_database(database) as conn:
                row=conn.execute("SELECT data FROM jobs WHERE id=?",(reply["job_id"],)).fetchone()
            if row:
                job=json.loads(row[0])
                if job.get("retry",{}).get("next_at"):
                    retry_state=job["retry"]
                if job["status"]=="failed": raise AssertionError(job["error"])
                if args.quotas and quota_hold is None and job.get("quota_waiting"):
                    assert job["status"]=="queued" and job["plan"],job
                    assert 0 < job["downloaded"] < len(payload),job
                    assert not job.get("retry"),job
                    parts=list((initial_data/"parts"/job["id"]).glob("*.part"))
                    saved={p:hashlib.sha256(p.read_bytes()).hexdigest() for p in parts}
                    count=len(requests)
                    time.sleep(.65)  # Observe a scheduler tick while the quota is exhausted.
                    assert len(requests)==count,"Quota hold restarted before reset"
                    assert all(hashlib.sha256(p.read_bytes()).hexdigest()==digest for p,digest in saved.items())
                    with open_profile_database(database) as conn:
                        used=conn.execute("SELECT hourly_used,daily_used FROM data_quotas").fetchone()
                        assert used[0] >= 1048576 and used[0]==used[1],used
                        # Expire only this isolated fixture's hour; the desktop must
                        # notice the reset and resume its saved ranges itself.
                        conn.execute("UPDATE data_quotas SET hour_start=?,hourly_limit=33554432 WHERE id=1",(int(time.time())-3600,))
                    quota_hold={"saved_bytes":job["downloaded"],"used_before_reset":used[0],"partial_files_unchanged":True}
                if job["status"]=="complete": break
            time.sleep(.05)
        assert job and job["status"]=="complete",job
        if args.quotas:
            assert quota_hold,"The download never reached its data quota"
            assert not job.get("quota_waiting"),job
            with open_profile_database(database) as conn:
                quota_usage=conn.execute("SELECT hourly_used,daily_used FROM data_quotas").fetchone()
            assert quota_usage[0]==len(payload)-quota_hold["saved_bytes"],quota_usage
            assert quota_usage[1]==quota_hold["used_before_reset"]+quota_usage[0],quota_usage
        transfer_seconds=time.monotonic()-transfer_started
        if args.bandwidth:
            assert payload_started,"No file body requested"
            payload_seconds=time.monotonic()-min(payload_started)
            assert payload_seconds >= len(payload)/(2*1024*1024)-.10,payload_seconds
            assert len(job["plan"]["segments"])==4,job
            with open_profile_database(database) as conn:
                assert conn.execute("SELECT bytes_per_second FROM bandwidth_settings WHERE id=1").fetchone()[0]==2097152
        if args.adaptive:
            layout=job["plan"]["segments"]
            assert isinstance(layout,dict) and layout["version"]==2,job
            assert len(layout["parts"])>4,job
            assert job["integrity"]["actual"]==hashlib.sha256(payload).hexdigest(),job
        if args.retry:
            assert retry_state and retry_state["attempts"]==1,retry_state
            assert len(requests)>=2 and requests[1]>=retry_state["next_at"],requests
            assert requests[1]-requests[0]>=5000,requests
            assert not job.get("retry"),job
        output=Path(job["saved_path"])
        assert Path(job["directory"])==expected_folder,job
        assert output.parent==expected_folder,output
        assert output.read_bytes()==payload,"Downloaded bytes differ"
        marker=Path(str(output)+":Zone.Identifier").read_text(encoding="utf-8")
        assert "[ZoneTransfer]" in marker and "ZoneId=3" in marker,"Missing Internet-origin marker"
        assert "http" not in marker,"Source URLs should not be copied into origin metadata"
        scan_report=None
        if has_antivirus:
            scan_deadline=time.monotonic()+25
            while time.monotonic()<scan_deadline:
                with open_profile_database(database) as conn:
                    row=conn.execute("SELECT data FROM antivirus_reports WHERE job_id=?",(job["id"],)).fetchone()
                if row:
                    scan_report=json.loads(row[0])
                    break
                time.sleep(.05)
            assert scan_report,"Automatic completion scan did not produce a report"
            assert scan_report["verdict"]==args.antivirus_expected,scan_report
            assert scan_report["sha256"]==hashlib.sha256(payload).hexdigest(),scan_report
            assert Path(scan_report["path"])==output,scan_report
            assert output.read_bytes()==payload,"Scan modified the downloaded file"
        # Close only the test process's window, exercising normal app shutdown.
        user32=ctypes.WinDLL("user32",use_last_error=True)
        user32.GetWindowThreadProcessId.argtypes=[wintypes.HWND,ctypes.POINTER(wintypes.DWORD)]
        user32.PostMessageW.argtypes=[wintypes.HWND,wintypes.UINT,wintypes.WPARAM,wintypes.LPARAM]
        callback_type=ctypes.WINFUNCTYPE(wintypes.BOOL,wintypes.HWND,wintypes.LPARAM)
        def close_owned(hwnd,_):
            pid=wintypes.DWORD()
            user32.GetWindowThreadProcessId(hwnd,ctypes.byref(pid))
            if pid.value==desktop.pid: user32.PostMessageW(hwnd,0x0010,0,0)
            return True
        callback=callback_type(close_owned)
        user32.EnumWindows.argtypes=[callback_type,wintypes.LPARAM]
        user32.EnumWindows(callback,0)
        assert desktop.wait(timeout=15)==0
        # Retry the same handoff with the desktop closed. The host must launch
        # a new desktop independently of its own short-lived process.
        cold=subprocess.Popen([str(binaries/"de-host.exe")],env=env,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE)
        cold_stdout,cold_stderr=cold.communicate(struct.pack("<I",len(data))+data,timeout=15)
        assert cold.returncode==0,cold_stderr.decode()
        cold_reply=json.loads(cold_stdout[4:])
        assert cold_reply["ok"] and cold_reply["message"]=="Added to the download queue",cold_reply
        command="Get-CimInstance Win32_Process -Filter 'ParentProcessId = %d' | Where-Object Name -eq 'de.exe' | Select-Object -ExpandProperty ProcessId" % cold.pid
        found=subprocess.run(["powershell.exe","-NoProfile","-Command",command],capture_output=True,text=True,timeout=15)
        child_pids=[int(line.strip()) for line in found.stdout.splitlines() if line.strip()]
        assert len(child_pids)==1,(found.stdout,found.stderr)
        child_pid=child_pids[0]
        kernel=ctypes.WinDLL("kernel32",use_last_error=True)
        kernel.OpenProcess.argtypes=[wintypes.DWORD,wintypes.BOOL,wintypes.DWORD]
        kernel.OpenProcess.restype=wintypes.HANDLE
        kernel.WaitForSingleObject.argtypes=[wintypes.HANDLE,wintypes.DWORD]
        kernel.CloseHandle.argtypes=[wintypes.HANDLE]
        kernel.TerminateProcess.argtypes=[wintypes.HANDLE,wintypes.UINT]
        child_handle=kernel.OpenProcess(0x00100001,False,child_pid)
        assert child_handle,"The launched desktop did not stay alive"
        try:
            deadline=time.monotonic()+15
            while time.monotonic()<deadline:
                def close_cold(hwnd,_):
                    pid=wintypes.DWORD()
                    user32.GetWindowThreadProcessId(hwnd,ctypes.byref(pid))
                    if pid.value==child_pid: user32.PostMessageW(hwnd,0x0010,0,0)
                    return True
                user32.EnumWindows(callback_type(close_cold),0)
                if kernel.WaitForSingleObject(child_handle,100)==0: break
            else:
                kernel.TerminateProcess(child_handle,1)
                raise TimeoutError("Cold-started desktop did not close normally")
        finally:
            kernel.CloseHandle(child_handle)
        synchronization=[]
        if args.synchronize:
            original_path=output
            original_bytes=payload
            previous_report=scan_report
            queue_id=str(uuid.uuid4())
            # Seed only the closed, isolated fixture profile. The desktop itself
            # must prepare/run each scheduled check and commit its result.
            with open_profile_database(database) as conn:
                conn.execute("INSERT INTO download_queues(id,name,max_active,running) VALUES(?,?,2,0)",(queue_id,"Synchronization smoke"))
                conn.execute("INSERT INTO synchronization_queues(queue_id) VALUES(?)",(queue_id,))
                conn.execute("UPDATE job_queues SET queue_id=? WHERE job_id=?",(queue_id,job["id"]))
            for expected in ["updated","up_to_date","failed"]:
                if expected=="updated":
                    payload=original_bytes+b"\nupdated synchronization fixture\n"
                    version='"desktop-smoke-v2"'
                sync_failure=expected=="failed"
                with open_profile_database(database) as conn:
                    conn.execute("UPDATE download_queues SET running=0,scheduled_at=? WHERE id=?",(int(time.time())-1,queue_id))
                desktop=subprocess.Popen([str(binaries/"de.exe")],env=env,stdin=subprocess.DEVNULL,stdout=stderr,stderr=stderr,startupinfo=startup)
                sync_deadline=time.monotonic()+25
                while time.monotonic()<sync_deadline:
                    assert desktop.poll() is None,"Desktop exited during synchronization"
                    with open_profile_database(database) as conn:
                        current=json.loads(conn.execute("SELECT data FROM jobs WHERE id=?",(job["id"],)).fetchone()[0])
                    sync=current.get("synchronization",{})
                    if expected=="failed" and current["status"]=="failed": break
                    if current["status"]=="failed": raise AssertionError(current)
                    if current["status"]=="complete" and sync.get("phase")==expected: break
                    time.sleep(.05)
                else: raise TimeoutError(f"Synchronization did not reach {expected}: {current}")
                assert original_path.read_bytes()==original_bytes,"The previous copy changed"
                if expected!="failed":
                    current_path=Path(current["saved_path"])
                    assert current_path.read_bytes()==payload
                    if expected=="updated":
                        assert current_path!=original_path
                        updated_path=current_path
                    else: assert current_path==updated_path,"Unchanged file was duplicated"
                    if has_antivirus:
                        while time.monotonic()<sync_deadline:
                            with open_profile_database(database) as conn:
                                row=conn.execute("SELECT data FROM antivirus_reports WHERE job_id=?",(job["id"],)).fetchone()
                            current_report=json.loads(row[0]) if row else None
                            if current_report and Path(current_report["path"])==current_path: break
                            time.sleep(.05)
                        else: raise TimeoutError("The updated copy did not receive an automatic scan")
                        assert current_report["verdict"]==args.antivirus_expected,current_report
                        assert current_report["sha256"]==hashlib.sha256(payload).hexdigest()
                        if expected=="up_to_date": assert current_report==previous_report,"An unchanged check repeated the scan"
                        previous_report=current_report
                count=len(requests)
                # Observe more than two scheduler intervals with the app alive.
                observe_until=time.monotonic()+1.2
                while time.monotonic()<observe_until:
                    assert len(requests)==count,"The queue repeated a completed check"
                    time.sleep(.05)
                synchronization.append({"result":expected,"previous_copy_preserved":True,"checked_at":sync.get("checked_at"),"scan":previous_report})
                user32.EnumWindows(callback,0)
                assert desktop.wait(timeout=15)==0
            assert updated_path.read_bytes()==payload,"A failed check changed the latest saved copy"
        result={"protocol":"automatic" if args.automatic else "manual","native_handoff":True,"desktop_completed":True,"normal_shutdown":True,"native_cold_start":True,"internet_origin_marker":True,"bytes":job["downloaded"],"sha256":job["integrity"]["actual"],"data_root":str(run_root)}
        if args.quotas:
            with open_profile_database(database) as conn:
                assert conn.execute("SELECT hourly_used,daily_used FROM data_quotas").fetchone()==quota_usage
            result["quotas"]={**quota_hold,"resumed_after_controlled_reset":True,"usage_persisted_after_cold_start":True,"hourly_used":quota_usage[0],"daily_used":quota_usage[1]}
        if synchronization: result["synchronization"]=synchronization
        result["folder_rules"]={"category":"Documents" if args.folders else "Other","custom":args.folders,"directory":str(expected_folder),"verified":True}
        if scan_report:
            result["antivirus"]={"provider":"malwarebytes" if args.antivirus_malwarebytes else "defender" if args.antivirus_defender else "fixture","mode":args.antivirus_mode,"report":scan_report,"automatic_completion":True,"download_bytes_unchanged":True}
        if args.retry:
            result["automatic_retry"]={"attempts":retry_state["attempts"],"observed_delay_ms":requests[1]-requests[0],"deadline_respected":True}
        if args.bandwidth:
            result["bandwidth"]={"global_bytes_per_second":2097152,"transfer_seconds":round(transfer_seconds,3),"payload_seconds":round(payload_seconds,3),"connections":4,"cap_respected":True}
            with open_profile_database(database) as conn:
                assert conn.execute("SELECT bytes_per_second FROM bandwidth_settings WHERE id=1").fetchone()[0]==2097152
        if args.adaptive:
            result["adaptive"]={"parts":len(job["plan"]["segments"]["parts"]),"requests":len(requests),"sha256_verified":True,"transfer_seconds":round(transfer_seconds,3)}
        if args.capture:
            screenshot=root/"artifacts"/"desktop-complete.png"
            env["EFRAME_SCREENSHOT_TO"]=str(screenshot)
            shot=subprocess.run([str(binaries/"de.exe")],env=env,stdin=subprocess.DEVNULL,stdout=stderr,stderr=stderr,startupinfo=startup,timeout=20)
            assert shot.returncode==0 and screenshot.exists()
            result["screenshot"]=str(screenshot)
        (run_root/"result.json").write_text(json.dumps(result,indent=2),encoding="utf-8")
        print(json.dumps(result))
    finally:
        if desktop.poll() is None:
            desktop.terminate()
            desktop.wait(timeout=10)
        server.shutdown()
        stderr.close()

if __name__=="__main__":
    main()

