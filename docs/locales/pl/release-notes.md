## 3.1.0 — 2026-10-27

### Szybsze pobieranie filmów i mniejszy narzut

Filmy pobierają do 4 segmentów równolegle, z uwzględnieniem niższych limitów globalnych i dla witryny. Wstrzymanie, ponowienie i restart aplikacji zachowują wszystkie zweryfikowane segmenty, również ukończone poza kolejnością; nieukończone są pobierane od nowa. Okno stanu pokazuje aktywne segmenty i połączenia. Buforowanie zapisu, rzadsze odświeżanie postępu i ponowne użycie połączenia z bazą ograniczają narzut. Wstrzymanie nadal czeka na zapis, a każdy odebrany blok jest trwale rozliczany w limicie danych.

### Czytelniejszy stan błędu

Etykieta błędu to teraz „Nieudane”; szczegóły błędu i automatyczne ponawianie nie zmieniły się.
