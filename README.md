# imgsort

**Organize photos into folders by capture date — dry-run, conflict policies, per-year summary. The EXIF reader and the calendar arithmetic are written from scratch. Rust standard library only.**

```
$ imgsort ~/Pictures --dest ~/Pictures/sorted --dry-run
move               /tmp/imgsort-src/2024/DSC_0003.jpg -> …/2024/2024-03-16/DSC_0003.jpg
move               /tmp/imgsort-src/DSC_0001.jpg       -> …/2024/2024-03-15/DSC_0001.jpg
move               /tmp/imgsort-src/DSC_0002.jpg       -> …/2026/2026-09-27/DSC_0002.jpg
move (file time)   /tmp/imgsort-src/plain.png           -> …/2026/2026-09-27/plain.png

dry run: 4 file(s) — 4 moved, 0 already sorted, 0 conflict(s) skipped, 0 undated
by year: 2024: 2  2026: 2
```

Ten thousand photos in `IMG_4821.jpg` form is the mess every photo
library accumulates. imgsort reads the capture date out of the file
itself — not the file's timestamp — and files each photo where a template
says it belongs.

## Why imgsort

- **EXIF read from scratch.** The JPEG marker walk to APP1, the TIFF
  header (both byte orders), the IFD entry table, the Exif sub-IFD
  pointer, inline-versus-offset ASCII values — all hand-written, with
  bounds checks at every step. The tests build real JPEG/EXIF byte
  sequences and read them back.
- **The right date.** `DateTimeOriginal` wins, then `DateTime`, then the
  file's modification time — and the output says which one it used
  (`move` vs `move (file time)`), so a wrongly dated batch is visible
  instead of silent.
- **Calendar math included.** Days-from-civil and its inverse are
  implemented directly (leap years, the 1900/2000 century rule and
  pre-epoch instants are all tested), so no date library is needed.
- **Dry-run first, always safe.** Nothing moves until you drop the flag;
  the plan is printed either way.
- **Conflict policies that mean something.** `rename` (the default)
  adds `-1`, `-2`, … within the same run *and* against files already on
  disk; `skip`, `overwrite` and `fail` are there for the other
  temperaments.
- **Templates you can read.** `{Y} {M} {D} {H} {m} {S} {name} {ext} {n}`
  with `/` making subdirectories, unknown placeholders left alone.
- **Summary, not just files.** Totals plus a per-year breakdown, as text
  or `--json`.

## Usage

```sh
imgsort ~/Pictures --dest ~/Sorted --dry-run
imgsort ~/Pictures --dest ~/Sorted --template '{Y}/{Y}-{M}/{name}.{ext}'
imgsort ~/Pictures --dest ~/Sorted --on-conflict skip
imgsort ~/Pictures --dest ~/Sorted --no-recursive --undated misc/
imgsort ~/Pictures --dest ~/Sorted --json | jq .by_year
```

Exit codes: 0 ok, 1 runtime error, 2 usage error.

## Development

```sh
cargo test --release
cargo clippy --all-targets -- -D warnings
```

## License

MIT — see [LICENSE](LICENSE).
