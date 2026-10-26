---
created: '2026-02-05'
parent: ./map.md
related:
- ./programarea.md
tags:
- formulare
- securitate
updated: '2026-04-20'
---
# Protecția formularelor împotriva spamului

Formularele de contact și de programare primeau zeci de mesaje automate pe zi.

## Câmpul ascuns

Fiecare formular are un câmp honeypot ascuns cu CSS. Un om nu îl vede și nu îl
completează; un robot îl completează. Mesajele cu câmpul completat sunt
aruncate fără eroare, ca robotul să nu afle de ce.

## Limitarea

Aceeași adresă IP poate trimite cel mult cinci formulare pe oră. Peste limită,
formularul afișează un mesaj politicos și nu trimite nimic.
