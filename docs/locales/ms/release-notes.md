## 3.10.1 — 2026-11-04

Had sumber penemuan rakan torrent, pembatalan dan penapisan alamat rangkaian ditambah baik. Storan kelayakan macOS dan Linux dikemas kini dengan mengekalkan log masuk sedia ada, dan penyimpanan folder torrent tidak lagi menimpa destinasi yang telah wujud.

[English details](../../../CHANGELOG.md#3101--2026-11-04)

## 3.10.0 — 2026-11-03

### macOS dan Linux

Pakej asli untuk Mac Apple Silicon dan Intel serta arkib Linux x64 menggunakan antara muka dan enjin muat turun yang sama seperti Windows. FFmpeg, sumber dan lesennya disertakan, bersama dialog asli dan stor kelayakan sistem operasi. Pakej Mac ditandatangani ad hoc, tanpa notarisasi Apple. Rujuk panduan untuk keperluan dan had ujian.

[Guide / English](../../cross-platform.md)

## 3.9.0 — 2026-11-01

### Aliran kerja torrent

Sokongan pautan magnet pelayar, seret dan lepas fail torrent serta cadangan papan klip. Perkongsian selepas selesai dan sambungan masuk ialah pilihan berasingan yang dimatikan secara lalai. Imbas torrent berbilang fail, pindah atau namakan semula folder; Hentikan semua turut menghentikan perkongsian.

[Guide / English](../../user-guide.md#download-torrents)

## 3.8.0 — 2026-11-01

### Buka torrent dari Windows

Buka fail .torrent dari Explorer atau pautan magnet dari pelayar untuk disemak. Pendaftaran Windows adalah pilihan, mengekalkan aplikasi lalai dan menyimpan semakan tertunda selepas mula semula. Perlindungan keselamatan torrent diperkukuh; magnet tanpa metadata dalam cache memerlukan penjejak, jika tidak gunakan fail .torrent. Ujian regresi ditambah untuk perlindungan konflik nama fail sedia ada.

## 3.7.0 — 2026-11-01

### Muat turun torrent

Muat turun torrent dan pautan magnet dengan pilihan fail, jeda, sambung semula dan pemulihan kemajuan yang disahkan. Torrent dimatikan secara lalai; aktifkan dan kawal muat naik dalam Keutamaan. Proksi menjeda torrent, dan muat naik berhenti apabila selesai. Termasuk pembaikan 3.6.1 dan pembantu torrent dalam pakej pemasangan.

### Penyepaduan keluaran

Panduan, README, laman web dan semakan pemasang telah dikemas kini. Selepas torrent pertama ditambah, profil memerlukan versi 3.7.0 atau lebih baharu supaya versi lama tidak mengubah tugas torrent. Mengaktifkan ciri sahaja tidak menaik taraf profil.

## 3.6.1 — 2026-10-31

### Pengurangan kebergantungan dan kelulusan sambungan

Penguncian fail terbina dalam Rust menggantikan fs2 dan membuang kebergantungan winapi lama, sambil mengekalkan tingkah laku kunci dan masa menunggu. Kemas kini sambungan yang menambah tugas, termasuk menggantikan ID tugas, kini memerlukan kelulusan walaupun sambungan dimatikan.

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
