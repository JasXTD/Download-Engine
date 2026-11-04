## 3.10.1 — 2026-11-04

### Segirezza e fidadad

Meglieradas las limitas da resursas per tschertgar peers torrent, l’annullaziun e la filtraziun da las adressas da rait. L’arcunaziun da las datas d’annunzia sin macOS e Linux è actualisada e mantegna las annunzias existentas. La memorisaziun d’ordinaturs torrent na remplazza betg pli destinaziuns existentas.

[English details](../../../CHANGELOG.md#3101--2026-11-04)

## 3.10.0 — 2026-11-03

### macOS e Linux

Pachets nativs per Mac cun Apple Silicon ed Intel ed in archiv Linux x64 dovran la medema interfatscha ed il medem motor da telechargiar sco Windows. FFmpeg, ses code da funtauna e sias licenzas èn inclus, ensemen cun dialogs nativs e la memorisaziun da las datas d’access dal sistem operativ. Ils pachets Mac èn signads ad hoc, senza notarissaziun dad Apple. Il manual descriva las pretensiuns ed ils cunfins dals tests.

[Guide / English](../../cross-platform.md)

## 3.9.0 — 2026-11-01

### Lavurar cun torrents

Sustegn per colliaziuns magnet dal navigatur, datotecas torrent tratgas en l’app e propostas da l’archiv provisoric. Cundivider suenter la fin e connexiuns entradas èn opziuns separadas, deactivadas sco standard. Ins po examinar pliras datotecas, spustar e renumnar ordinaturs; Fermar tut ferma era la cundivisiun.

[Guide / English](../../user-guide.md#download-torrents)

## 3.8.0 — 2026-11-01

### Avrir torrents da Windows

Avri datotecas .torrent da l’Explorer e colliaziuns magnet dal navigatur per las controllar. La registraziun facultativa en Windows mantegna las apps da standard; las controllas pendentas restan suenter ina nova aviertura. La protecziun da torrents è vegnida rinforzada: magnets senza metadatas en il cache dovran in tracker, uschiglio duvrai ina datoteca .torrent. Tests da regressiun controllan la protecziun existenta cunter conflicts da nums da datotecas.

## 3.7.0 — 2026-11-01

### Telechargiar torrents

Datotecas torrent e colliaziuns magnet cun selecziun da datotecas, pausa, cuntinuaziun e recuperaziun da datas verifitgadas. Deactivà sco standard; activar e controllar il transferiment en las preferenzas. In proxy activ metta ils torrents en pausa; il transferiment sa ferma suenter la fin. Cuntegna las correcturas da 3.6.1 ed il program auxiliar per torrents.

### Integraziun da la versiun

Actualisà il mussavia, README, la pagina web e las controllas dal program d’installaziun. Suenter avair agiunt l’emprim torrent dovra il profil la versiun 3.7.0 u pli nova, per impedir che versiuns pli veglias modifitgeschian las incumbensas torrent. Activar mo la funcziun n’actualisescha betg il profil.

## 3.6.1 — 2026-10-31

### Main dependenzas ed approvaziun d'extensiuns

Ils bloccadis da datotecas integrads en Rust remplazzan fs2 ed elimineschan la veglia dependenza winapi. Il cumportament dals bloccadis e da la spetga resta il medem. Actualisaziuns d'extensiuns che agiuntan incumbensas, era cun remplazzar lur ID, dovran ussa in'approvaziun, er sche l'extensiun è deactivada.

## 3.6.0 — 2026-10-31

### Extensiuns

La nova pagina Extensiuns remplazza Scripts ed ha ils tabs Installadas, Scripts, Reglas e Cronologia d'execuziuns. Ina extensiun è ina suletta datoteca .de-extension.json che unescha lecturs da websites, furniturs da telechargiada ed incumbensas d'automatisaziun. L'installaziun è mo pussaivla ord ina datoteca locala; ils producents na vegnan betg verifitgads. Ina verificaziun mussa tge che l'extensiun po far, ella cumenza deactivada ed ina actualisaziun sto dumandar avant che survegnir access nov. Ses access al web è limità als hosts declarads. Suenter installar l'emprima extensiun na pon versiuns veglias betg pli scriver en quest profil.

## 3.5.2 — 2026-10-30

### Tschertgar actualisaziuns a l’avertura

Activà sco standard: ina controlla mintga giada che l’app vegn averta. Ina nova versiun permetta da telechargiar l’actualisaziun u spetgar. Sbagls da rait n’avran nagina fanestra. La controlla po vegnir deactivada en las preferenzas. Telechargiai ed installai actualisaziuns da la pagina uffiziala da versiuns sin GitHub.

## 3.5.1 — 2026-10-29

### Segirezza dals furniturs Lua

- Resolve/refresh e transform/finish vegnan exequids en process cun dretgs separads. Il process d’elavuraziun na survegn mai `xd.fetch`, er betg durant chargiar il code. Duvrai `xd.phase` per l’inizialisaziun tenor fasa; las variablas localas dal resolver na vegnan betg transferidas.
- Ils plans memorisads cuntegnan mo grondezza, hash ed identitad da la transfurmaziun. Ils URL vegnan renovads tenor basegn; il code fixà per l’incumbensa vegn protegì cun DPAPI da l’utilisader Windows actual. La migraziun atomara mantegna ils lioms dals puncts da cuntinuaziun e blochescha scritturas da versiuns veglias.
- `xd.key` e `scripts protect-key`, che legia la clav da stdin redirigì, porschan referenzas opacas a clavs AES liadas al furnitur. Clavs secretas na ston betg esser en il code u en exports. Copias da segirezza veglias e clavs gia scrittas en scripts da la biblioteca na vegnan betg stizzadas; vesair `docs/lua-providers.md`.

### Stizzar la cronologia

Avant da stizzar la cronologia vegn mussada ina conferma cun la colonna ed il dumber da telechargiadas terminadas. Interrumper u serrar mantegna la cronologia. Las datotecas telechargiadas vegnan mantegnidas.

## 3.5.0 — 2026-10-29

### Lua Provider API 1

Furniturs Lua determineschan las funtaunas, elavuran bytes e verifitgeschan ils hash cun pausa e cuntinuaziun. Mintga telechargiada conserva sia versiun memorisada dal script. Ils tests dovran mo datotecas criptadas generadas localmain. Suenter memorisar in furnitur na pon versiuns veglias betg pli scriver en quest profil.

## 3.4.1 — 2026-10-29

La glista reducescha copiar e tschertgar repetidamain en grondas colonnas. Fanestras pitschnas laschan dapli spazi per nums da datotecas. La colonna po vegnir aviada u messa en pausa sper ses status; aviar surscriva l'urari. Mussar sin il status renda visiblas explicaziuns da novas emprovas e sbagls. Transferiment, verificaziun e format da cuntinuaziun restan invariads.

## 3.4.0 — 2026-10-29

### Collecziun da temas

Preferenzas → Aspect → Persunalisà porscha Neon Courier, Reactor Night, Aurora e Moonveil. Adatta il fund e l’accent u importa PNG/JPEG. Memorisar mantegna la prevista; Annullar restabilescha il tema precedent. Ils maletgs restan locals, cun limits da grondezza ed elavuraziun en il fund.

### Meglieraziuns da profils e controllas

Inclus èn era protecziun da versiuns dal profil e controllas d’access MCP.
