## 3.18.1 — 2026-11-08

- Il chargiar si è deactivà sco standard. Vus al pudais activar en Preferenzas → Torrents.
- **Permetter connexiuns d’entrada?** Auters peers pon sa connectar cun questa app. Quai augmenta l’exposiziun a la rait. Las configuraziuns dal firewall e dal router restan invariadas.
- **Activar UPnP?** Durant che torrents èn activs, vegn il router dumandà da rediriger il port TCP dal torrent. Auters peers pon alura eventualmain cuntanscher l’app via l’internet. Memorisai las preferenzas per applitgar questa midada.

[English details](../../../CHANGELOG.md#3181--2026-11-08)

## 3.18.0 — 2026-11-08

### VPN / interfatscha da rait

- Liar torrents ad ina adressa d’interfatscha. Ils transferiments sa ferman sche ella svanescha; autras telechargiadas mantegnan lur configuraziun.
- DNS dovra er questa interfatscha. Duvra la medema versiun IP sco l’adressa tschernida. La redirecziun dals ports dal router è deactivada durant la colliaziun.
- TCP / uTP · Criptà / Text cler

[English details](../../../CHANGELOG.md#3180--2026-11-08)

## 3.17.0 — 2026-11-08

### Criptaziun tranter peers, uTP e funtaunas web

Agiuntà opziuns per la criptaziun MSE/PE tranter peers, colliaziuns uTP e funtaunas web HTTP/HTTPS. Tut las trais èn deactivadas sco standard e pon vegnir activadas en las preferenzas da torrents. Las parts vegnan verifitgadas e las funtaunas memorisadas per cuntinuar suenter in restart. Meglierà la pausa, la cuntinuaziun e la funcziun per fermar tut. La criptaziun na zuppenta betg l’adressa IP.

[English details](../../../CHANGELOG.md#3170--2026-11-08)

## 3.16.1 — 2026-11-08

### Correcturas da las configuraziuns e da la controlla

Las configuraziuns d’extensiuns memoriseschan ussa text vid e l’entir formular en ina suletta transacziun. Midadas betg memorisadas restan tar il midar extensiun. Il buttun per memorisar resta visibel en fanestras pitschnas; actualisaziuns chargian da nov las valurs da standard. Ils menus per fermar torrents cundividids e l’indicaziun da las limitas da sveltezza èn curregids. Ins ha reducì scrittiras repetidas dal progress e curregì ils texts davart l’export dal rapport, la protecziun dal sistem e las actualisaziuns.

[English details](../../../CHANGELOG.md#3161--2026-11-08)

## 3.16.0 — 2026-11-07

### Extensiuns Lua d’exempel e controlla da permissiuns

Cun questa versiun vegnan furnidas tschintg extensiuns Lua: datotecas publicas dad Internet Archive, originals da Wikimedia Commons, datotecas da versiuns GitHub, ordinaziun da datotecas terminadas tenor tip e notificaziuns a la fin da la colonna cun l’opziun da lantschar la proxima. Tschernair las extensiuns sut Extensions → Examples e controllar las permissiuns avant l’installaziun. Il code furnì na vegn betg exequì automaticamain. La pagina vida, la controlla d’installaziun e las permissiuns per incumbensa èn pli cleras. L’access a datotecas po vegnir concedì u revocà suenter l’installaziun; in mussavia declera l’emprima extensiun.

Pachets modifitgads dovran ina nova controlla. Ils identificaturs reservads per exempels pretendan datotecas identicas a quellas furnidas; caracters da controlla che pon engianar vegnan refusads en las metadatas. Las extensiuns vegnan anc adina tschertgadas en l’ordinatur plugins da l’applicaziun. Traducziuns mancantas dals rapports da collaps e da l’interfatscha èn cumplettadas, la tschertga da texts da translatar è meglierada ed il rumantsch è unifitgà. Notizias istoricas mancantas èn restabilidas; ils pachets macOS/Linux cuntegnan ils mussavias e maletgs colliads per la documentaziun senza connexiun.

[English details](../../../CHANGELOG.md#3160--2026-11-07)

## 3.15.0 — 2026-11-06

### Nuschel dals providers Lua e suttitels ASS

HTTP en Lua sustegna ussa HEAD, PUT, PATCH, DELETE ed OPTIONS, cuntegns binars cun limitas da grondezza e sessiuns furnidas explicitamain, liadas ad ina suletta derivanza. Las datas d’access restan criptadas ordaifer il resolver e na vegnan ni memorisadas en ils plans ni transmessas als processurs offline. Ils plans da medias permettan segments cun ina grondezza nunenconuschenta; lua_stream elavura blocs limitads cun puncts da controlla verifitgads per pausa, cuntinuaziun, fermada e recuperaziun.

Agiunt l’import da datas d’access binaras/RSA protegidas dal sistem operativ, bytes casuals, SHA-1/SHA-256, HMAC, suttascripziuns e decifrar cun RSA sco er decifrar blocs AES. Ils suttitels ASS mantegnan stils ed instrucziuns da formataziun integradas en MKV; MP4 als transfurma en suttitels da text. Las novas funcziuns per datas d’access e medias dovran in’app che sustegna la versiun da scriver profils 9. Ils formats existents dals puncts da controlla restan cumpatibels.

Il laboratori CENC offline per auturs dovra Lua per analisar ina part limitada da la criptaziun da samples AVC/AAC, simular in barat da licenza signà, transmetter clavs e cumbinar las pistas decifradas cun suttitels ASS. L’autentificaziun da paginas, ils formats da licenza ed ils parsers da containers restan incumbensa dal code Lua. Ils tests dovran datas generadas, senza servetschs u contos reals; quai n’è betg in’implementaziun cumpletta da CENC.

### Cundivisiun e tschertga da torrents

La cundivisiun d’in torrent terminà sa ferma ussa automaticamain tar la proporziun 2.0 u suenter in’emna, tut tenor tge che arriva l’emprim. Quai è activà sco standard, er per profils existents; omaduas limitas sa laschan midar u deactivar en Preferenzas → Torrents. Chargiar si durant la telechargiada quinta er per la proporziun. La fanestra da status mussa la proporziun, ed in torrent fermà d’ina limita mantegna sias datotecas e porscha Cundivider danovamain en ses menu.

Las colliaziuns magnet senza tracker na vegnan betg pli simplamain refusadas. La fanestra Agiuntar telechargiada declera ch’ils auters computers vesan tar ina tschertga en la DHT tge torrent che Vus vulais e Vossa adressa IP, e tschertga pir suenter in clic sin Tschertgar en la DHT. Voss computer vegn pir annunzià sco peer, cura che la glista da datotecas verifitgada mussa in torrent public. In torrent privat ferma la tschertga, ed ina tschertga finescha suenter 10 minutas. Colliaziuns da navigaturs, da l’archiv provisoric u da l’Explorer na cumenzan mai da sasez ina tschertga.

Nov è l’opziun Configurar il router (UPnP). Ella è deactivada sco standard, sco las connexiuns entradas, da las qualas ella dependa. Ella communitgescha mo cun il gateway da standard da la rait privata, dumonda ina suletta regla per renviar il port, valaivla per ina ura a la giada, ed allontanescha mo sia atgna regla; sin macOS n’è ella betg disponibla. Peers e computers da la DHT chattads sin ports d’auters servetschs (sut 1024 e ports sco SIP 5060) na vegnan mai contactads. Telechargiadas fermadas per in motiv da la DHT mussan ussa il motiv en la lingua tschernida.

[English details](../../../CHANGELOG.md#3150--2026-11-06)

## 3.10.2 — 2026-11-04

### Rapports locals da collaps

Agid e protocols permettan da leger, copiar ed exportar rapports da Rust panic e da serradas nunregularas. Nagina transmissiun automatica; ils rapports na cuntegnan nagins pleds-clav, URLs da telechargiada, percurs persunals ni dumps da memoria.

[English details](../../../CHANGELOG.md#3102--2026-11-04)

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

## 3.3.3 — 2026-10-28

### Meglieraziuns da l’interfatscha

L’editur ed ils scripts chargiads han ussa registers separads. Tschertga u creescha scripts en la glista ed avra l’editur cumplet en il chau da l’editur Lua u PowerShell. Memorisar, exequir/fermar ed ils protocols opziunals restan sper il code. Ils cumonds da datotecas èn en il menu d’acziuns da l’editur. La midada da vista mantegna il code betg memorisà, ils parameters e la posiziun dal cursor.

### Acziuns da telechargiar pli reactivas

L’actualisaziun da las colonnas e dal status, las dumondas da video dal navigatur e l’agiunta da telechargiadas vegnan elavuradas en il fund per che la memoria lenta na blocchia betg ils clics. Ils dialogs mussan il progress, impedeschan tramissiuns dublas e mantegnan il sboz sche memorisar na reussescha betg. Er ils ordinaturs vegnan averts en il fund; resultats vegls na remplazzan betg la colonna selecziunada da nov.
