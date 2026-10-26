# Download Engine

## Emprims pass

Las preferenzas sa avran en ina fanestra cun categorias. Memorisar applitgescha las midadas; Interrumper, × u Escape sbittan midadas betg memorisadas. Il tema po vegnir previsualisà; Restabilir pertutga mo la categoria actuala. Ils editurs avanzads memoriseschan separadamain e returnan a las preferenzas suenter serrar.

### Emprims pass

Tscherna Agiuntar URL, encolla ina colliaziun directa e verifitga il num da datoteca, la colonna e l’ordinatur. Avia lura la telechargiada. Agiuntar ina gruppa accepta ina colliaziun per lingia. Tenor in model generescha colliaziuns numeradas sco img001…img100 ord ina colliaziun cun *. Sut Parameters → Apparientscha → Lingua tscherna Rumantsch Grischun u Lingua dal sistem. La selecziun automatica reconuscha rm-CH; linguas betg sustegnidas dovran englais. La tscherna vala immediatamain e vegn memorisada.

## Telechargiadas e recuperaziun

### Telechargiadas e recuperaziun

Clicca duas giadas sin ina telechargiada per vesair progress, sveltezza e temp restant. Pausa e Fermar conservan las datas parzialas. Aviar immediatamain dat prioritad a las datotecas tschernidas; ils limits da l’app e las quotas da datas valan vinavant. Per ina colliaziun scadida dovra Actualisar la colliaziun da telechargiada en il menu contextual. La recuperaziun verifitgescha las datas memorisadas avant cuntinuar. Allontanar las terminadas stizza mo endataziuns da la cronologia e conserva las datotecas. Incumbensas → Exportar la glista… memorisescha tias telechargiadas en ina datoteca, sco copia da segirezza u per in auter PC. Incumbensas → Importar ina glista… mussa las novitads avant che agiuntar ellas; colliaziuns existentas vegnan sursiglidas e novas telechargiadas restan en pausa. Vista → Zona da laschar crudar mobila mussa ina pitschna zona che resta sur las autras fanestras; lascha crudar colliaziuns sin ella per las telechargiar, era cur che Download Engine è en la zona da notificaziun.

## Websites offline

### Websites offline

Il Grabber chatta datotecas telechargiablas u memorisescha in website offline. Definescha la pagina iniziala, profunditad, limit da paginas e limit da grondezza. Verifitga las datotecas chattadas avant las agiuntar a la colonna. Avra index.html en la copia memorisada per navigar offline. Scripts, formulars, annunzia e reproducturs integrads èn deactivads en la copia statica. Il rapport da copia inditgescha las resursas mancontas.

## Integraziun dal navigatur

### Integraziun dal navigatur

Avra Utensils → Configuraziun dal navigatur per colliar Chrome, Edge, Brave u Firefox. Chrome, Edge e Brave basegnan l’ID real da l’extensiun installada. Dovra Verifitgar la connexiun en il popup da l’extensiun. Opziuns dal navigatur → Lingua da l’extensiun porscha ina tscherna independenta da la lingua da l’app. Il buttun da video avra la selecziun da qualitad en l’app. L’annunzia dal website vegn duvrada mo per la telechargiada permessa.

## Colonnas ed uraris

### Colonnas ed uraris

Administrar las colonnas gruppescha datotecas e limitescha quants telechargiadas che pon funcziunar a la giada. Novas colonnas spetgan in cumenzament manual u in urari. Definescha temps da cumenzament e pausa e dis da repetiziun en la Planisaziun. L’app sto restar averta, era en la zona da notificaziun. Sortida automatica, stizzar, durmir ed ibernaziun dal computer basegnan ina tscherna explicita; ti pos interrumper il quint enavos.

## Scripts

### Scripts

Sut Utensils → Scripts creescha reglas tenor website u extensiun da datoteca. L’emprima regla d’attribuziun correspundenta vala. En Scripts pos ti duvrar Lua u PowerShell. Memorisescha las midadas avant exequir. Ils eveniments cumpiglian execuziun manuala, cumenzament da l’app, interval e fin da telechargiada u colonna. Activescha e memorisescha l’eveniment automatic per al duvrar. Fermar il script terminescha l’execuziun senza revocar las acziuns gia transmessas a l’app. Guarda ils protocols d’execuziun per resultats e sbagls.

## Scripts Lua

### Scripts Lua

Lua 5.4 è integrà: ti na stos betg l’installar separadamain. Tscherna Nov script, lingua Lua ed eveniment Mo manual. Encolla l’exempel sutvart, memorisescha ed exequescha. En ils protocols duessas ti vesair Hello from Lua!, l’eveniment ed ils nums da las colonnas. xd.queues() returna las colonnas; xd.downloads() returna las telechargiadas. event.downloads cuntegna las datotecas terminadas per l’eveniment da fin; event.queue descriva la colonna per ses eveniment. Ils nums da las APIs e dals champs restan per englais. L’API cumpletta è en docs/automation.md. Na scriva betg secrets en ils protocols. Lua dovra in ambient limità; PowerShell ha ils dretgs da tes conto Windows.

Ils editurs Lua e PowerShell verifitgeschan automaticamain la sintaxa suenter ina curta pausa da scriver, senza memorisar u exequir il script. Nov èn numers da lingia, colurs da sintaxa e sbagls cliccabels. Scriva il cumenzament d’in num u smatga Ctrl+Space per propostas: tscherna cun las frizzas, inserescha cun Tab, Enter u in clic e serra cun Esc. Las cumplettaziuns pon vegnir revocadas; sbagls durant l’execuziun restan en il protocol.

```lua
print("Hello from Lua!")
print("Trigger:", event.trigger)
for _, queue in ipairs(xd.queues()) do
    print(queue.name, "running:", queue.running)
end
for _, file in ipairs(event.downloads) do
    print("Finished:", file.name or file.id)
end
```

L'eveniment Telechargiada betg reussida exequescha in script, sch'ina telechargiada basegna anc attenziun suenter las empruvas automaticas da Download Engine; event.downloads cuntegna lura l'error da mintga telechargiada. xd.retry(id) ed xd.restart(id) emprovan anc ina giada (in script po far quai 5 giadas per telechargiada), xd.notify(title, text) mussa in avis da Windows, ed xd.store.get ed xd.store.set memoriseschan valurs tranter las execuziuns. xd.rename, xd.move ed xd.remove midan la datoteca mo, sche Po midar datotecas telechargiadas è activà. Exequir cun in exempel… emprova in script d'eveniment cun las telechargiadas che ti tschernas; las midadas vegnan mo simuladas.

```lua
for _, file in ipairs(event.downloads) do
    if pcall(xd.retry, file.id) then
        print("Retrying:", file.name or file.id)
    else
        xd.notify("Download still failing", (file.name or file.id) .. ": " .. (file.error or "no details"))
    end
end
local handled = (xd.store.get("handled") or 0) + #event.downloads
xd.store.set("handled", handled)
print("Failed downloads handled:", handled)
```

Per modifitgar in script en in auter program, dovra Avrir ina datoteca… e Memorisar en ina datoteca… en il chau da l'editur. Avrir ina datoteca… copiescha ina datoteca .lua u .ps1 en l'editur e dumonda avant, sch'ella remplazzass code betg memorisà; tscherna Memorisar il script per al tegnair. Memorisar en ina datoteca… scriva il code da l'editur en ina datoteca.

## Lingia da commandos

### Lingia da commandos

Avra PowerShell en l’ordinatur da l’applicaziun ed emprova quests commandos per examinar. L’app sto esser activa per retschaiver ils commandos da controlla. Nums da commandos, identificaturs e JSON restan per englais. Metta percurs cun spazis en virgulettas. Guarda docs/automation.md per agiuntar gruppas e controllar colonnas.

```powershell
.\de-cli.exe --help
.\de-cli.exe queues
.\de-cli.exe list
```

## Integraziun MCP

### Integraziun MCP

Sin la pagina MCP avia il server HTTP local e copiescha la configuraziun dal client en tes client IA. Cumenza cun Mo leger. Il client basegna l’URL ed il chau privat Authorization. Na cundivida betg il token. Permetter midadas activescha acziuns en l’app, era l’execuziun da scripts memorisads. Fermar il server scollegia ils clients; telechargiadas gia aviadas cuntinueschan. Guarda docs/mcp-ai-guide.md per duvrar ils utensils.

MCP s’avra ussa en ina fanestra separada sur la glista d’utensils, il menu Utensils u las Preferenzas. La pagina actuala resta visibla. Serrar cun × u Escape na ferma betg il server e mantegna las midadas MCP betg memorisadas. Fermar il server per separar ils clients IA. Sche la fanestra è vegnida averta ord las Preferenzas, maina serrar enavos al medem sboz.

## Controlla antivirus

Scanner: Microsoft Defender, Malwarebytes, Kaspersky, Bitdefender, ClamAV.

Basegna l'integraziun Explorer da antivirus per englais. Il termin limita la transmissiun, betg la controlla. Interrumpa controllas en antivirus.

Il ordinatur d’installaziun standard è necessari.

L’antivirus controllescha la protecziun e la quarantina. Ina dumonda na conferma betg l’absenza da smanatschas.

Verifitgar la configuraziun: Integraziun chattada; la disponibilitad dal scan vegn verifitgada tar la dumonda.

### Controlla antivirus

Avra Utensils → Controlla antivirus. Singul tscherna ina telechargiada terminada, Gruppa pliras datotecas. Ina scannada singula po vegnir interrutta. Ils tabs èn bloccads durant la scannada u la controlla en Malwarebytes / Kaspersky / Bitdefender.

Tscherna il program da controlla sut Parameters ed avra lura Utensils → Controlla antivirus → Gruppa. Tscherna las datotecas terminadas, avia e verifitga ils resultats. Las datotecas vegnan controlladas ina suenter l’autra. Fermar la gruppa interrumpa las dumondas restantas. Cun Malwarebytes / Kaspersky / Bitdefender spetga la fin da la controlla e verifitga il resultat en sia app avant cuntinuar cun la proxima datoteca. Ferma en Malwarebytes / Kaspersky / Bitdefender ina controlla gia transmessa. Ina dumonda transmessa na cumprova betg che la datoteca è senza smanatschas.

## Parameters e protocols

### Parameters e protocols

Ils Parameters cumpiglian lingua, tema, ordinaturs, sveltezza, quotas, connexiuns per datoteca e proxy. Protocols permetta da tschertgar, filtrar, copiar ed exportar eveniments. URLs, percurs, resultats da scripts e sbagls tecnics conservan lur text original. Verifitga ils rapports avant als cundivider. Las instrucziuns curtas èn disponiblas offline; las referenzas tecnicas extendidas e la cronologia precedenta restan per englais.

La versiun installada dumonda a Windows l’ordinatur actual da telechargias a mintga aviada, era suenter ina midada sin in auter disc. Novas telechargias dovran quel lieu. Telechargias existentas, progress memorisà ed ordinaturs tschernids na vegnan betg midads. La versiun portabla determinescha l’ordinatur tenor il lieu da l’app a mintga aviada. Profils vegls mantegnan il num IDM EX. Sche il lieu na po betg vegnir determinà, vegn mussà in sbagl.

## Solvir problems

### Solvir problems

Sche la connexiun dal navigatur na funcziuna betg, controlla che l’app saja averta, che l’ID da l’extensiun saja correct e ch’il navigatur saja registrà. Suenter l’actualisaziun rechargia l’extensiun e la pagina. Per sbagls da telechargiada controlla il spazi liber, ils dretgs da l’ordinatur, il proxy e la validitad da la colliaziun. Emprova da cuntinuar avant stizzar datas parzialas. Sche segns mancan, installescha il sustegn da scrittiras per la lingua en Windows e reavia l’app.
