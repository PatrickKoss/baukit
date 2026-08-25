#!/usr/bin/env bash
set -euo pipefail

usage() {
    printf 'Usage: %s --target <product-dir> [--claude] [--codex] [--copy]\n' "$0"
}

die() {
    printf 'error: %s\n' "$*" >&2
    exit 1
}

target=
select_claude=false
select_codex=false
explicit_harness=false
copy_mode=false

while (($# > 0)); do
    case $1 in
        --target)
            (($# >= 2)) || die '--target requires a directory'
            target=$2
            shift 2
            ;;
        --claude)
            select_claude=true
            explicit_harness=true
            shift
            ;;
        --codex)
            select_codex=true
            explicit_harness=true
            shift
            ;;
        --copy)
            copy_mode=true
            shift
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            usage >&2
            die "unknown argument: $1"
            ;;
    esac
done

[[ -n $target ]] || {
    usage >&2
    die '--target is required'
}
[[ -d $target ]] || die "target directory does not exist: $target"

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
skills_dir=$script_dir/skills
[[ -d $skills_dir ]] || die "canonical skills directory is missing: $skills_dir"
target=$(CDPATH= cd -- "$target" && pwd)

if [[ $explicit_harness == false ]]; then
    [[ -d $target/.claude ]] && select_claude=true
    [[ -d $target/.agents ]] && select_codex=true
    if [[ $select_claude == false && $select_codex == false ]]; then
        die 'neither .claude nor .agents exists under the target; pass --claude and/or --codex to create one'
    fi
fi

skill_sources=()
for skill_source in "$skills_dir"/*; do
    [[ -d $skill_source && -f $skill_source/SKILL.md ]] || continue
    skill_sources+=("$skill_source")
done
((${#skill_sources[@]} > 0)) || die "no canonical skills found in $skills_dir"

destinations=()
[[ $select_claude == true ]] && destinations+=("$target/.claude/skills")
[[ $select_codex == true ]] && destinations+=("$target/.agents/skills")

is_owned_destination() {
    local destination=$1
    local source=$2
    local skill_name=$3
    local marker=

    if [[ -L $destination ]]; then
        [[ $(readlink -- "$destination") == "$source" ]]
        return
    fi
    if [[ -d $destination && -f $destination/.baukit-agent-skill ]]; then
        IFS= read -r marker <"$destination/.baukit-agent-skill" || true
        [[ $marker == "baukit-agent-skill:$skill_name" ]]
        return
    fi
    return 1
}

# Refuse all collisions before modifying any harness directory.
for skills_destination in "${destinations[@]}"; do
    for skill_source in "${skill_sources[@]}"; do
        skill_name=${skill_source##*/}
        destination=$skills_destination/$skill_name
        if [[ -e $destination || -L $destination ]]; then
            is_owned_destination "$destination" "$skill_source" "$skill_name" ||
                die "refusing to overwrite non-Baukit skill destination: $destination"
        fi
    done
done

installed=0
for skills_destination in "${destinations[@]}"; do
    mkdir -p -- "$skills_destination"
    for skill_source in "${skill_sources[@]}"; do
        skill_name=${skill_source##*/}
        destination=$skills_destination/$skill_name
        if [[ -e $destination || -L $destination ]]; then
            rm -rf -- "$destination"
        fi
        if [[ $copy_mode == true ]]; then
            cp -R -- "$skill_source" "$destination"
            printf 'baukit-agent-skill:%s\n' "$skill_name" >"$destination/.baukit-agent-skill"
            method=copied
        else
            ln -s -- "$skill_source" "$destination"
            method=linked
        fi
        printf '%s %s -> %s\n' "$method" "$skill_name" "$destination"
        ((installed += 1))
    done
done

printf 'Installed %d Baukit skill destination(s) across %d harness(es).\n' \
    "$installed" "${#destinations[@]}"
