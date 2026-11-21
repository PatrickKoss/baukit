# shellcheck shell=bash
# Sourced by release.sh and promote.sh. Commits GitOps pins and pushes them to main.
# In CI, GITOPS_DEPLOY_KEY holds the private deploy key that branch protection lets push to main.

github_host_key='github.com ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl'
push_attempts=3
gitops_key_directory=""

gitops_cleanup_key() {
  [[ -z $gitops_key_directory ]] || rm -rf -- "$gitops_key_directory"
}

# Sets gitops_remote, and with a deploy key points GIT_SSH_COMMAND at it.
gitops_select_remote() {
  if [[ -z ${GITOPS_DEPLOY_KEY:-} ]]; then
    gitops_remote=origin
    return
  fi
  [[ -n ${GITHUB_REPOSITORY:-} ]] || {
    printf 'GITHUB_REPOSITORY must name owner/repository when GITOPS_DEPLOY_KEY is set\n' >&2
    return 2
  }
  gitops_key_directory=$(mktemp -d)
  (umask 077 && printf '%s\n' "$GITOPS_DEPLOY_KEY" >"$gitops_key_directory/deploy-key")
  printf '%s\n' "$github_host_key" >"$gitops_key_directory/known_hosts"
  export GIT_SSH_COMMAND="ssh -i $gitops_key_directory/deploy-key -o IdentitiesOnly=yes -o StrictHostKeyChecking=yes -o UserKnownHostsFile=$gitops_key_directory/known_hosts"
  gitops_remote="git@github.com:${GITHUB_REPOSITORY}.git"
}

# gitops_commit_and_push <repository root> <message> <path>...
gitops_commit_and_push() {
  local root=$1 message=$2 attempt
  shift 2
  git -C "$root" add -- "$@"
  git -C "$root" -c user.name="${GITOPS_COMMIT_NAME:-github-actions[bot]}" \
    -c user.email="${GITOPS_COMMIT_EMAIL:-41898282+github-actions[bot]@users.noreply.github.com}" \
    commit --quiet -m "$message"
  if [[ ${dry_run:-false} == true ]]; then
    printf '+ git push HEAD:main (dry run; the commit stays local)\n' >&2
    return 0
  fi
  gitops_select_remote || return
  for ((attempt = 1; attempt <= push_attempts; attempt++)); do
    if git -C "$root" push "$gitops_remote" HEAD:main; then
      gitops_cleanup_key
      return 0
    fi
    printf 'push rejected, rebasing onto the current main (attempt %s of %s)\n' "$attempt" "$push_attempts" >&2
    git -C "$root" fetch "$gitops_remote" main
    git -C "$root" rebase FETCH_HEAD
  done
  gitops_cleanup_key
  printf 'could not push "%s" to main\n' "$message" >&2
  return 1
}
