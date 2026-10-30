## 3.6.0 — 2026-10-31

### Sambungan

Halaman Sambungan baharu menggantikan Skrip, dengan tab Dipasang, Skrip, Peraturan dan Sejarah pelaksanaan. Sambungan ialah satu fail .de-extension.json yang menggabungkan pembaca laman, penyedia muat turun dan tugas automasi. Pasang daripada fail setempat sahaja; penerbit tidak disahkan. Semakan menunjukkan perkara yang boleh dilakukannya, sambungan bermula dalam keadaan dimatikan, dan kemas kini mesti meminta kebenaran sebelum mendapat akses baharu. Akses webnya terhad kepada hos yang diisytiharkan. Selepas sambungan pertama dipasang, versi lama tidak boleh menulis profil ini.

## 3.5.2 — 2026-10-30

### Semak kemas kini semasa pelancaran

Diaktifkan secara lalai dan menyemak sekali setiap kali aplikasi dibuka. Versi baharu menawarkan pilihan kemas kini atau kemudian. Ralat luar talian tidak membuka dialog. Boleh dimatikan dalam Keutamaan. Muat turun dan pasang kemas kini dari halaman keluaran GitHub rasmi.

## 3.5.1 — 2026-10-29

### Keselamatan penyedia Lua

- Resolve/refresh dan transform/finish berjalan dalam proses pekerja dengan keupayaan berasingan. Pemproses tidak menerima `xd.fetch`, termasuk semasa memuatkan kod. Gunakan `xd.phase` untuk pemulaan mengikut fasa; pemboleh ubah setempat resolver tidak dipindahkan.
- Pelan tersimpan hanya menyimpan saiz, hash dan identiti transformasi. URL yang diperoleh diperbaharui apabila diperlukan; salinan kod tugas dilindungi oleh DPAPI pengguna Windows semasa. Peningkatan tugas lama berlaku secara atomik, mengekalkan ikatan titik semak dan menyekat penulis versi lama.
- `xd.key` dan `scripts protect-key`, yang membaca kunci daripada stdin yang dialihkan, menyediakan pemegang kunci AES legap yang terikat kepada penyedia. Kunci peribadi tidak perlu dimasukkan dalam kod atau eksport. Sandaran lama dan literal kunci sedia ada dalam pustaka tidak dipadamkan; lihat `docs/lua-providers.md`.

### Kosongkan sejarah

Mengosongkan sejarah yang selesai kini meminta pengesahan dan memaparkan baris gilir serta jumlahnya. Batal atau tutup untuk mengekalkan sejarah. Fail yang dimuat turun dikekalkan.

## 3.5.0 — 2026-10-29

### Lua Provider API 1

Penyedia muat turun Lua kini boleh menentukan sumber, memproses bait dan mengesahkan hash dengan kawalan jeda/sambung biasa. Setiap muat turun menyimpan semakan skripnya. Contoh diuji dengan fail terenkripsi yang dijana secara setempat sahaja. Selepas penyedia disimpan, versi lama tidak boleh menulis profil ini.

## 3.4.1 — 2026-10-29

Senarai muat turun mengurangkan penyalinan dan kerja carian berulang untuk baris gilir besar. Tetingkap kecil memberi lebih ruang kepada nama fail. Mulakan atau jeda baris gilir di sebelah statusnya; memulakannya mengatasi jadual. Tuding pada status untuk penjelasan cubaan semula dan ralat. Pemindahan, pengesahan dan format sambung semula tidak berubah.

## 3.4.0 — 2026-10-29

### Koleksi tema

Keutamaan → Penampilan → Tersuai menyediakan Neon Courier, Reactor Night, Aurora dan Moonveil. Laraskan latar dan warna aksen atau import PNG/JPEG. Simpan mengekalkan pratonton; Batal memulihkan tema asal. Imej kekal setempat, dengan had saiz dan pemuatan latar.

### Penambahbaikan profil dan kawalan

Termasuk perlindungan versi profil dan kawalan akses MCP.
