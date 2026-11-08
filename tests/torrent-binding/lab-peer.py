"""Offline Docker fixture. Standard library only; never connects out or executes downloads."""
import hashlib
import http.server
import json
import os
import socket
import socketserver
import ssl
import struct
import threading

ROOT = '/work/fixtures'
os.makedirs(ROOT, exist_ok=True)
LOCK = threading.Lock()
DATA = b'Download Engine harmless network fixture\n'
PAYLOAD = (DATA * (4 * 1024 * 1024 // len(DATA) + 1))[:4 * 1024 * 1024]

def record(kind, local, remote, detail):
    with LOCK, open('/work/traffic.jsonl', 'a', encoding='utf8') as log:
        log.write(json.dumps(dict(kind=kind, local=local, remote=remote, detail=detail)) + '\n')

def bencode(value):
    if isinstance(value, int): return b'i' + str(value).encode() + b'e'
    if isinstance(value, bytes): return str(len(value)).encode() + b':' + value
    if isinstance(value, list): return b'l' + b''.join(map(bencode,value)) + b'e'
    return b'd' + b''.join(bencode(k) + bencode(v) for k,v in sorted(value.items())) + b'e'

piece = 32768
info = {b'name': b'harmless.fixture', b'length':len(PAYLOAD), b'piece length':piece, b'pieces':b''.join(hashlib.sha1(PAYLOAD[i:i+piece]).digest() for i in range(0,len(PAYLOAD),piece)), b'private':1}
with open(ROOT+'/harmless.fixture','wb') as f: f.write(PAYLOAD)
for name, extra in [('peer',{}),('webseed',{b'url-list':[b'http://fixture.test:8080/harmless.fixture']}),('tracker',{b'announce':b'http://fixture.test:8080/announce'}),('udp-tracker',{b'announce':b'udp://fixture.test:6969/announce'})]:
    with open(ROOT+'/'+name+'.torrent','wb') as f: f.write(bencode({b'info':info,**extra}))
with open(ROOT+'/manifest.json','w') as f: json.dump(dict(size=len(PAYLOAD),sha256=hashlib.sha256(PAYLOAD).hexdigest()),f)

class HTTP(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_): pass
    def do_GET(self):
        record('http',self.server.server_address[0],self.client_address[0],self.path)
        if self.path.startswith('/announce'):
            body=bencode({b'interval':30,b'peers':socket.inet_aton('172.30.241.3')+struct.pack('!H',6881)})
        else: body=PAYLOAD if self.path=='/harmless.fixture' else DATA
        response=200
        content_range=None
        if 'Range' in self.headers:
            begin,end=map(int,self.headers['Range'].removeprefix('bytes=').split('-'))
            content_range=f'bytes {begin}-{end}/{len(body)}'
            body=body[begin:end+1];response=206
        self.send_response(response)
        self.send_header('Content-Length',str(len(body)))
        if content_range: self.send_header('Content-Range',content_range)
        self.end_headers()
        try: self.wfile.write(body)
        except (BrokenPipeError,ConnectionResetError): pass

def dns_reply(data, tcp, local, remote):
    if len(data)<17: return b''
    end=12;labels=[]
    while end<len(data) and data[end]:
        count=data[end];end+=1
        if count>63 or end+count>len(data): return b''
        labels.append(data[end:end+count].decode('ascii'));end+=count
    end+=1
    if end+4>len(data):return b''
    name='.'.join(labels);kind=struct.unpack('!H',data[end:end+2])[0]
    question=data[12:end+4]
    record('dns-tcp' if tcp else 'dns-udp',local,remote,name)
    truncated=name=='tcp.fixture.test' and not tcp
    answer=(b'\xc0\x0c'+struct.pack('!HHIH',1,1,30,4)+socket.inet_aton('172.30.241.3')) if kind==1 and not truncated else b''
    return data[:2]+struct.pack('!HHHHH',0x8380 if truncated else 0x8180,1,bool(answer),0,0)+question+answer

def udp(local,port):
    sock=socket.socket(socket.AF_INET,socket.SOCK_DGRAM);sock.bind((local,port))
    while True:
        data,remote=sock.recvfrom(65536)
        if port==53: reply=dns_reply(data,False,local,remote[0])
        elif port==6969 and len(data)>=16:
            action,transaction=struct.unpack('!II',data[8:16]);record('tracker-udp',local,remote[0],action)
            reply=struct.pack('!IIQ',0,transaction,7) if action==0 else struct.pack('!IIIII',1,transaction,30,0,1)+socket.inet_aton('172.30.241.3')+struct.pack('!H',6881)
        else: record('udp',local,remote[0],data.decode('ascii',errors='replace'));reply=data
        if reply:sock.sendto(reply,remote)

class DNS(socketserver.BaseRequestHandler):
    def handle(self):
        stream=self.request.makefile('rwb');prefix=stream.read(2)
        if len(prefix)!=2:return
        data=stream.read(struct.unpack('!H',prefix)[0])
        reply=dns_reply(data,True,self.server.server_address[0],self.client_address[0])
        stream.write(struct.pack('!H',len(reply))+reply);stream.flush()

servers=[]
for address in ['172.30.241.3','172.30.240.3']:
    for port in [53,9000,9001,6969]:threading.Thread(target=udp,args=(address,port),daemon=True).start()
    servers.extend([http.server.ThreadingHTTPServer((address,8080),HTTP),socketserver.ThreadingTCPServer((address,53),DNS)])
tls=http.server.ThreadingHTTPServer(('172.30.241.3',8443),HTTP)
context=ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
context.load_cert_chain('/work/fixture-cert.pem','/work/fixture-key.pem')
tls.socket=context.wrap_socket(tls.socket,server_side=True);servers.append(tls)
for server in servers: threading.Thread(target=server.serve_forever,daemon=True).start()
threading.Event().wait()
