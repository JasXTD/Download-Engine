## 3.16.0 — 2026-11-07

### Sambungan permulaan Lua dan kawalan kebenaran

Lima sambungan Lua disertakan: fail awam Internet Archive, fail asal Wikimedia Commons, aset keluaran GitHub, pengisihan fail siap mengikut jenis, serta pemberitahuan giliran selesai dengan pilihan memulakan giliran seterusnya. Pilih melalui Extensions → Examples dan semak kebenaran sebelum memasang; sambungan tidak berjalan hanya kerana disertakan. Halaman kosong, semakan pemasangan, kebenaran setiap tugas dan kawalan akses fail selepas pemasangan diperjelas. Panduan menulis sambungan pertama turut ditambah.

Semakan yang lapuk ditolak jika pakej berubah. ID contoh terpelihara memerlukan fail yang sama tepat dengan salinan disertakan, dan metadata menolak aksara kawalan yang mengelirukan. Penemuan kekal dalam folder plugins aplikasi. Terjemahan laporan ranap dan UI dilengkapkan, pengesanan teks terjemahan diperbaiki, bahasa Romansh diseragamkan, nota keluaran lama dipulihkan, dan pakej macOS/Linux menyertakan panduan serta imej yang dipautkan untuk bacaan luar talian.

[English details](../../../CHANGELOG.md#3160--2026-11-07)

## 3.15.0 — 2026-11-06

### Teras pembekal Lua dan sari kata ASS

HTTP Lua kini menyokong HEAD, PUT, PATCH, DELETE dan OPTIONS, badan binari dengan had saiz serta sesi yang dibekalkan secara nyata dan terikat pada satu asal. Kelayakan kekal disulitkan di luar penyelesai, tidak disimpan dalam pelan dan tidak dihantar kepada pemproses luar talian. Pelan media menerima saiz segmen yang belum diketahui; lua_stream memproses blok terhad dengan titik semak yang disahkan untuk Jeda, Sambung, Henti dan pemulihan.

Ditambah import kelayakan binari/RSA yang dilindungi sistem operasi, bait rawak, SHA-1/SHA-256, HMAC, tandatangan dan penyahsulitan RSA serta penyahsulitan blok AES. Sari kata ASS mengekalkan gaya dan arahan gaya sebaris dalam MKV, manakala MP4 menukarkannya kepada sari kata teks. Ciri kelayakan dan media baharu memerlukan aplikasi yang menyokong versi penulis profil 9; format titik semak sedia ada kekal serasi.

Makmal pengarang CENC luar talian menggunakan Lua untuk menghuraikan subset terhad penyulitan sampel AVC/AAC, pertukaran lesen bertandatangan sintetik, penyerahan kunci dan penggabungan trek yang dinyahsulit dengan sari kata ASS. Pengesahan laman, format lesen dan penghurai bekas media kekal dalam Lua. Ujian menggunakan data janaan tanpa perkhidmatan atau akaun sebenar; ini bukan pelaksanaan CENC yang lengkap.

### Perkongsian dan carian torrent

Perkongsian torrent yang telah selesai kini berhenti sendiri pada nisbah 2.0 atau selepas seminggu, mengikut mana yang dicapai dahulu. Ciri ini dihidupkan secara lalai, termasuk untuk profil sedia ada; kedua-dua had boleh diubah atau dimatikan dalam Keutamaan → Torrent. Muat naik semasa memuat turun turut dikira dalam nisbah. Tetingkap status menunjukkan nisbah, dan torrent yang dihentikan oleh had mengekalkan failnya serta menawarkan Kongsi semula dalam menunya.

Pautan magnet tanpa penjejak tidak lagi ditolak begitu sahaja. Tetingkap Tambah muat turun menerangkan bahawa carian DHT membolehkan komputer lain melihat torrent yang anda mahu dan alamat IP anda, dan hanya mencari selepas anda mengklik Cari dalam DHT. Komputer anda hanya diumumkan sebagai rakan setelah senarai fail yang disemak menunjukkan torrent awam. Torrent peribadi menghentikan carian, dan carian tamat selepas 10 minit. Pautan daripada pelayar, papan klip atau Explorer tidak sekali-kali mencari dengan sendirinya.

Baharu: Sediakan penghala saya (UPnP), dimatikan secara lalai seperti sambungan masuk yang diperlukannya. Ia hanya berhubung dengan get laluan lalai rangkaian peribadi, meminta satu peraturan pemajuan yang sah selama sejam pada satu masa dan hanya membuang peraturannya sendiri; macOS melaporkannya sebagai tidak tersedia. Rakan dan nod DHT yang ditemui pada port perkhidmatan lain (di bawah 1024, serta port seperti SIP 5060) tidak pernah dihubungi. Muat turun yang berhenti atas sebab DHT kini memaparkan sebabnya dalam bahasa yang dipilih.

[English details](../../../CHANGELOG.md#3150--2026-11-06)

## 3.10.2 — 2026-11-04

### Laporan ranap setempat

Bantuan dan Log kini menyediakan laporan Rust panic dan penutupan tidak normal untuk disemak, disalin atau dieksport. Tiada muat naik automatik; kata laluan, URL muat turun, laluan peribadi dan dump memori tidak disertakan.

[English details](../../../CHANGELOG.md#3102--2026-11-04)

## 3.10.1 — 2026-11-04

### Keselamatan dan kebolehpercayaan

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

## 3.3.3 — 2026-10-28

### Penambahbaikan antara muka

Editor dan skrip yang dimuatkan kini mempunyai tab berasingan. Cari atau cipta skrip dalam pustaka, dan buka editor penuh dari pengepala editor Lua atau PowerShell. Simpan, Jalankan/Hentikan dan log pilihan kekal bersama kod. Arahan fail berada dalam menu tindakan editor. Pertukaran paparan mengekalkan kod yang belum disimpan, tetapan dan kedudukan kursor.

### Tindakan muat turun lebih responsif

Kemas kini baris gilir dan status, permintaan video pelayar serta penambahan muat turun diproses di latar belakang supaya storan perlahan tidak menyekat klik. Dialog menunjukkan kemajuan, menghalang penghantaran berganda dan mengekalkan draf jika simpanan gagal. Folder dibuka di latar belakang dan hasil kemas kini lama tidak menggantikan pilihan baris gilir baharu.
