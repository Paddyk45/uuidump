# uuidump
small program for scraping minecraft uuids from mojangs api using https://mowojang.matdoes.dev

help (pass -h):
```
-w, --wordlist-path=WORDLIST  [path] the file to pull the names from. all non-mc-name characters
                         will be nuked.
-t, --threads=THREADS    [num] how many threads to spawn for making requests.
-o, --output=OUTPUT     [path] where to output uuids to.
-i, --ignored-uuids=IGNORED  [path] which uuids to ignore if found. useful in combination with
                         one of mats uuid dumps. if not given, don't ignore any uuids.
-r, --ignored-truncation=IGNORED_TRUNCATION  [num] amount of hex digits to keep from from the
                         ignored uuids (8 for laby). no truncation if not given.
-s, --suffixes=SUFFIXES  [path] list of suffixes to append to each word in the wordlist. words
                         with no suffixes will not be kept. no suffixing if not given.
-a, --print-ignored      whether to print ignored uuids in a gray color.
```

examples:
```sh
uuidump -w users.txt -t 200 -o found.txt # scrape `users.txt` with 200 threads and output them to `found.txt`.
uuidump -w users.txt -i ignores.txt -o found.txt # ignore all uuids from `ignores.txt`.
uuidump -w users.txt -i truncated_uuids.txt -r 8 -o found.txt # ignore using laby uuid hashes (collisions will lose results!).
uuidump -w users.txt -s suffixes.txt -o found.txt # apply all suffixes in `suffixes.txt` to every word in wordlist.
```

demo:
[![asciicast](https://asciinema.org/a/bMHT7TYXJTTjsKETeamKCBioe.svg)](https://asciinema.org/a/bMHT7TYXJTTjsKETeamKCBioe)
