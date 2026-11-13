# Define criteria-based song selection semantics

> Migrated to [Codeberg issue #3](https://codeberg.org/padi/izmusica/issues/3). This is a local snapshot; update the Codeberg issue.

Type: grilling
Status: resolved

## Question

Given the agreed criteria (not-yet-played, artist, album, genre, year, duration, and file location) and AND combination in v1, what filter operators, missing-metadata behavior, sorting, and selection-to-queue interaction should the spec require? Keep advanced OR/grouped rules and saved rule sets out of the first version unless the user reopens that scope.

## Answer

Criteria combine with AND; text matching ignores case and diacritics but displays original metadata. Year and duration support exact and inclusive range filters, with year based on album/single release year. Missing artist, album, genre, or year values require an explicit “is missing” criterion; unknown play count is zero. Every credited artist is searchable, with each track shown once under its first-billed artist. Results sort by artist, album release date, then track number; singles are one-track albums and tracks without album identity group under “No Album.” Users can append selected or all results in displayed order; duplicate queue entries are allowed. See [Codeberg issue: Define criteria-based song selection semantics](https://codeberg.org/padi/izmusica/issues/3).
