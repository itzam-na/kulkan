# Issue tracker: Codeberg

Issues for this repo live in Codeberg Issues at https://codeberg.org/padi/izmusica. Use the `gt` CLI, authenticated to Codeberg, with `padi/izmusica` as the repository.

## Issue operations

- **Publish an issue:** `gt issue create --repo padi/izmusica --title "..." --body "..."`
- **Fetch an issue:** `gt issue view NUMBER --repo padi/izmusica` (add `--comments` for conversation)
- **Update or comment:** use `gt issue edit` or `gt issue comment`
- **Close or reopen:** use `gt issue close` or `gt issue reopen`
- **Local files:** `.scratch/` may hold specs and research assets, but Codeberg Issues are the canonical tracker. Link or attach relevant assets to their issue.

PRs are not part of the triage queue by default.

## Wayfinding operations

- **Map:** one Codeberg issue labeled `wayfinder:map`; its body contains the Destination, Notes, Decisions so far, Not yet specified, and Out of scope sections.
- **Ticket:** one Codeberg issue labeled `wayfinder:<type>` (`research`, `prototype`, `grilling`, or `task`). Include the map's URL in the ticket body. Refer to issues by title, with their links.
- **Blocking:** use Codeberg's native issue dependencies, managed with `gt issue edit --add-blocked-by` / `--add-blocks`.
- **Frontier:** find open, unassigned tickets linked to the map and without open blockers.
- **Claim:** assign the ticket to the driver with `gt issue edit NUMBER --add-assignee @me` before work.
- **Resolve:** add the answer as an issue comment, close the ticket, and update the map issue's Decisions so far with a linked title and one-line gist.
- **Research assets:** keep substantial findings in `.scratch/<effort>/research/` and link or attach them from the ticket.
