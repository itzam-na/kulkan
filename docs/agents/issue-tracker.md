# Issue tracker: GitHub

Issues and specs for this repo live in GitHub Issues at https://github.com/itzam-na/kulkan. Use the `gh` CLI with `itzam-na/kulkan` as the repository.

## Issue operations

- **Publish an issue:** `gh issue create --repo itzam-na/kulkan --title "..." --body "..."`
- **Fetch an issue:** `gh issue view NUMBER --repo itzam-na/kulkan` (add `--comments` for conversation)
- **Update or comment:** use `gh issue edit` or `gh issue comment`
- **Close or reopen:** use `gh issue close` or `gh issue reopen`
- **Local files:** `.scratch/` may hold specs and research assets, but GitHub Issues are the canonical tracker. Link or attach relevant assets to their issue.

PRs are not part of the triage queue by default.

## Wayfinding operations

- **Map:** one GitHub issue labeled `wayfinder:map`; its body contains the Destination, Notes, Decisions so far, Not yet specified, and Out of scope sections.
- **Ticket:** one GitHub issue labeled `wayfinder:<type>` (`research`, `prototype`, `grilling`, or `task`). Include the map's URL in the ticket body. Refer to issues by title, with their links.
- **Blocking:** use GitHub's native issue dependencies, managed with `gh issue create --blocked-by` or `gh issue edit --add-blocked-by`.
- **Frontier:** find open, unassigned tickets linked to the map and without open blockers.
- **Claim:** assign the ticket to the driver with `gh issue edit NUMBER --add-assignee @me` before work.
- **Resolve:** add the answer as an issue comment, close the ticket, and update the map issue's Decisions so far with a linked title and one-line gist.
- **Research assets:** keep substantial findings in `.scratch/<effort>/research/` and link or attach them from the ticket.
