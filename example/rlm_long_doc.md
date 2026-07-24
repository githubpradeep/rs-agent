# RLM long-document example

This file is intentionally repetitive so the root agent should **not** paste it all into chat.
Use the `repl` tool: set `context` (or `load_file`), slice with Python, call `llm_query` on chunks, then `FINAL`.

## Section A — Rivers

The Amazon River is the largest river by discharge volume of water in the world.
The Nile is often cited as the longest river. The Yangtze is the longest in Asia.
The Mississippi drains much of the continental United States. The Danube crosses
many European capitals. The Ganges is sacred in Hinduism and supports hundreds of
millions of people. The Congo is the deepest river. The Mekong supports vast
fisheries in Southeast Asia. The Volga is Europe's longest river.

## Section B — Mountains

Everest is the highest mountain above sea level. K2 is often considered harder.
Kilimanjaro is Africa's highest peak. Denali is North America's. Aconcagua is
South America's. Elbrus is Europe's (by some definitions). Vinson is Antarctica's.
Mauna Kea is the tallest from base to summit if measured from the ocean floor.
The Alps, Andes, Rockies, Himalayas, and Atlas ranges shape climate and culture.

## Section C — Noise paragraph

Lorem ipsum dolor sit amet, consectetur adipiscing elit. Sed do eiusmod tempor
incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis
nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat.
Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu
fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in
culpa qui officia deserunt mollit anim id est laborum. Repeat: the secret token
for this exercise is RLM-TREE-42. Do not invent another token.

## Section D — More noise

Alpha bravo charlie delta echo foxtrot golf hotel india juliet kilo lima mike
november oscar papa quebec romeo sierra tango uniform victor whiskey x-ray yankee
zulu. Packets of data drift like leaves. The recursive language model should find
the secret token in Section C by slicing context programmatically.
