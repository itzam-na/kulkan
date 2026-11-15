# Define library scanning and media-file identity behavior

> Migrated to [Codeberg issue #5](https://codeberg.org/itzamna/kulkan/issues/5). This is a local snapshot; update the Codeberg issue.

Type: grilling
Status: resolved

## Question

For audio indexed from user-selected folders, what should happen when files are added, changed, moved, renamed, deleted, duplicated across folders, or reached through symbolic links? Decide how rescans update the library and whether play history should follow a moved or replaced file.

## Answer

The catalog has one entry per resolved path; overlapping roots collapse to one path, but copies at different paths remain separate. Scans are explicit. Nested symlinks are skipped; an explicitly selected directory symlink is followed. Unreadable roots/subtrees preserve prior entries and report an incomplete scan. Missing files and removed roots become inactive records with history retained until explicitly cleared. Tag-only changes preserve history; changed audio content starts a new Track identity and the old identity remains recoverable. A unique exact audio-content match across a move/rename carries history; ambiguous matches and re-encodes do not. See [Codeberg issue: Define library scanning and media-file identity behavior](https://codeberg.org/itzamna/kulkan/issues/5).
