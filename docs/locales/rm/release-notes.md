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
