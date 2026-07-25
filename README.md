# idle civilizations

A world of civilizations that plays itself in a small window. There is nothing to click.

![screenshot](screenshot.png)

Peoples grow, discover, spread along their coasts, fight over borders, split apart when
they get too big, and die. You watch. Close it, come back tomorrow, and the world has
moved on without you.

```
cargo run --release
```

`space` pause &nbsp;·&nbsp; `+` / `-` speed &nbsp;·&nbsp; `esc` quit &nbsp;·&nbsp; `--fresh` new world &nbsp;·&nbsp; `--shot out.bmp` save a picture

## how the idle part works

The save file is one line: `<seed> <year>`. The simulation is deterministic, so resuming
is just replaying from the seed — there is no serializer and no world state on disk. The
years that passed while you were away come from the save file's modification time, capped
at 5000 so a week off doesn't fast-forward an eternity.

## how the world stays interesting

Getting a world that *keeps having history* was the whole problem. Three separate versions
looked fine for a few thousand years and then locked solid forever:

- **tech had to stop feeding people.** With tech linear in carrying capacity, one cell
  eventually feeds millions, a people squeezed to its last province becomes unconquerable,
  and nothing ever dies again.
- **the size penalty had to be quadratic.** A linear one means shrinking makes a realm
  *stronger*, so every world settles into a crystal of equal, immortal statelets.
- **battles had to be decisive.** With linear odds every frontier stays mushy and the map
  never consolidates into anything worth looking at.

The test asserts the world is still turning over at year 40,000 — that some civilization
alive at the end was born in the second half of history.
