// Independent, deterministic MEGA-format vectors using Node's built-in crypto.
// No network, downloaded code, real links, user data, or additional packages.
const { createCipheriv, createHash } = require('node:crypto');
const fs = require('node:fs');
const path = require('node:path');
const key = Buffer.from('000102030405060708090a0b0c0d0e0f', 'hex');
const nonce = Buffer.from('1011121314151617', 'hex');
const encrypt = (mode, bytes, iv) => {
  const cipher = createCipheriv('aes-128-' + mode, key, iv);
  cipher.setAutoPadding(false);
  return Buffer.concat([cipher.update(bytes), cipher.final()]);
};
const sha = bytes => createHash('sha256').update(bytes).digest('hex');
const vectors = [0, 1, 15, 16, 17, 131071, 131072, 131073, 393216, 3670016, 7340061].map(size => {
  const plain = Buffer.from(Array.from({length:size}, (_, i) => (i * 13 + 7) % 251));
  let folded = Buffer.alloc(16), offset = 0, length = 131072;
  while (offset < size) {
    const count = Math.min(size - offset, length);
    const chunk = Buffer.alloc(Math.ceil(count / 16) * 16);
    plain.copy(chunk, 0, offset, offset + count);
    const mac = encrypt('cbc', chunk, Buffer.concat([nonce, nonce])).subarray(-16);
    folded = encrypt('ecb', Buffer.from(folded.map((byte, i) => byte ^ mac[i])), null);
    offset += count;
    length = Math.min(1048576, length + 131072);
  }
  const mac = Buffer.alloc(8);
  mac.writeUInt32BE((folded.readUInt32BE(0) ^ folded.readUInt32BE(4)) >>> 0, 0);
  mac.writeUInt32BE((folded.readUInt32BE(8) ^ folded.readUInt32BE(12)) >>> 0, 4);
  const tail = Buffer.concat([nonce, mac]);
  const fileKey = Buffer.concat([Buffer.from(key.map((b, i) => b ^ tail[i])), tail]).toString('base64url');
  const data = encrypt('ctr', plain, Buffer.concat([nonce, Buffer.alloc(8)]));
  const name = 'fixture.bin';
  const attr = Buffer.from('MEGA' + JSON.stringify({n:name}));
  const padded = Buffer.alloc(Math.ceil(attr.length / 16) * 16);
  attr.copy(padded);
  return {size, key:fileKey, attributes:encrypt('cbc', padded, Buffer.alloc(16)).toString('base64url'),
    sha256:sha(plain), encrypted_sha256:sha(data), prefix:data.subarray(0, 32).toString('hex')};
});
fs.writeFileSync(path.join(__dirname, 'vectors.json'), JSON.stringify(vectors, null, 2) + '\n');
