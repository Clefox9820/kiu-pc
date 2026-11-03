#!/usr/bin/env bash
# Fails when any commit named on stdin (one id per line) holds something from
# the game: a dump, the code 3dsrecomp translates from it, what that
# compiles into, game audio or icons, build output, pictures or videos, or
# anything big enough to be one of them. Each commit's whole tree is read,
# since a subtree squash commit can carry files the merged tree drops.
# File names are only ever data here: read NUL-separated, matched, printed.
set -euo pipefail
shopt -s nocasematch

dumps='\.(3ds|cci|cia|cxi|cxa|nds|app|3dsx|cro|crs|sav|bin|exe|dll|so|a|o|obj|lib|pdb|res|hints|keys|state|wav|bcstm|bcwav|ico)$'
translated='(^|/)code/.*\.c$'
built='^(tools|build)/|(^|/)play\.bat$'
pictures='\.(png|jpe?g|gif|webp|bmp|mp4|webm)$'
limit=2097152

problems=$(mktemp)
trap 'rm -f "$problems"' EXIT

while IFS= read -r commit; do
    [ -n "$commit" ] || continue
    if ! git cat-file -e "$commit^{commit}" 2>/dev/null; then
        echo "error: $commit is not a commit in this clone" >&2
        exit 2
    fi
    while IFS= read -r -d '' entry; do
        meta=${entry%%$'\t'*}
        path=${entry#*$'\t'}
        size=${meta##* }
        why=
        if [[ $path =~ $dumps ]]; then why="dump, binary or game asset"
        elif [[ $path =~ $translated ]]; then why="translated code"
        elif [[ $path =~ $built ]]; then why="made by setup.ps1"
        elif [[ $path =~ $pictures ]]; then why="picture or video; diagrams go in as SVG"
        elif [[ $size =~ ^[0-9]+$ ]] && [ "$size" -gt "$limit" ]; then why="over 2 MB ($size bytes)"
        fi
        if [ -n "$why" ]; then printf '%s\t%s\t%s\n' "$path" "$why" "${commit:0:12}" >> "$problems"; fi
    done < <(git ls-tree -r -z -l "$commit")
done

if [ -s "$problems" ]; then
    echo "These files can't be in the repository (path, why, a commit that has it):"
    sort -t $'\t' -k1,1 -u "$problems"
    exit 1
fi
echo "No game files."
