# Presets

A preset starts one session or several, on one machine or many, in one go.

```toml
[presets.ml]
description = "Train on b31, review here"
layout = "grid"                  # tabs | grid | columns | rows | herdr

[[presets.ml.panes]]
name = "train"
host = "b31"                     # or "tag:gpu", "local"
agent = "claude"
dir = "~/work/hunt"
worktree = "auto"                # none | auto | <branch>
prompt = "Run the sweep in plan.md"
```

```sh
aleph preset ls · aleph preset show ml
aleph preset launch ml [--dry-run]
aleph preset save NAME host:session…    # from sessions you have running
aleph preset import FILE|URL            # shows what would start where, then asks
```

In the dashboard, `P` picks a preset; with sessions marked, `s` saves them as one.
