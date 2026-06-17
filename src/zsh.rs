pub fn init_script() -> &'static str {
    r#"# Waymark zsh integration.

z() {
  emulate -L zsh
  local dest
  dest="$(command waymark query --kind dir --best -- "$@")" || return
  [[ -n "$dest" ]] || return 1
  builtin cd -- "$dest"
}

zz() {
  emulate -L zsh
  local dest
  dest="$(command waymark query --kind dir --interactive -- "$@")" || return
  [[ -n "$dest" ]] || return 1
  builtin cd -- "$dest"
}

f() {
  emulate -L zsh
  command waymark query --kind file --best -- "$@"
}

v() {
  emulate -L zsh
  local file
  file="$(command waymark query --kind file --best -- "$@")" || return
  _waymark_edit_file "$file"
}

vv() {
  emulate -L zsh
  local file
  file="$(command waymark query --kind file --interactive -- "$@")" || return
  _waymark_edit_file "$file"
}

_waymark_edit_file() {
  emulate -L zsh
  local file="$1"
  [[ -n "$file" ]] || return 1

  local -a editor
  editor=("${(@z)${EDITOR:-vi}}")
  "${editor[@]}" -- "$file"
}

d() {
  emulate -L zsh
  command waymark query --kind dir --best -- "$@"
}

a() {
  emulate -L zsh
  command waymark query --kind any --best -- "$@"
}

s() {
  emulate -L zsh
  command waymark query --kind any --score -- "$@"
}

sf() {
  emulate -L zsh
  command waymark query --kind file --score -- "$@"
}

sd() {
  emulate -L zsh
  command waymark query --kind dir --score -- "$@"
}

_waymark_chpwd() {
  emulate -L zsh
  [[ -o interactive ]] || return 0
  [[ -n "${_waymark_hook_active:-}" ]] && return 0

  local _waymark_hook_active=1
  command waymark add --kind dir -- "$PWD" >/dev/null 2>&1 || true
  return 0
}

_waymark_preexec() {
  emulate -L zsh
  [[ -o interactive ]] || return 0
  [[ -n "${_waymark_hook_active:-}" ]] && return 0

  local -a waymark_words waymark_files
  waymark_words=("${(z)1}")
  (( ${#waymark_words} >= 2 )) || return 0

  local waymark_cmd="${(Q)waymark_words[1]}"
  waymark_cmd="${waymark_cmd:t}"
  case "$waymark_cmd" in
    vim|nvim|vi|code|less|more|cat|bat|open|xdg-open) ;;
    *) return 0 ;;
  esac

  local waymark_i waymark_arg waymark_after_double_dash=0
  for (( waymark_i = 2; waymark_i <= ${#waymark_words}; waymark_i++ )); do
    waymark_arg="${(Q)waymark_words[waymark_i]}"
    if [[ "$waymark_arg" == "~" ]]; then
      waymark_arg="$HOME"
    elif [[ "$waymark_arg" == "~/"* ]]; then
      waymark_arg="$HOME/${waymark_arg#\~/}"
    fi
    if [[ "$waymark_arg" == "--" ]]; then
      waymark_after_double_dash=1
      continue
    fi
    [[ "$waymark_after_double_dash" == 0 && "$waymark_arg" == -* ]] && continue
    [[ -f "$waymark_arg" ]] || continue
    waymark_files+=("$waymark_arg")
  done

  (( ${#waymark_files} )) || return 0
  local _waymark_hook_active=1
  command waymark add --kind file -- "$waymark_files[@]" >/dev/null 2>&1 || true
  return 0
}

_waymark_comma_complete() {
  emulate -L zsh
  local cur kind query
  cur="${words[CURRENT]}"

  case "$cur" in
    f,*) kind="file"; query="${cur#f,}" ;;
    d,*) kind="dir"; query="${cur#d,}" ;;
    ,*) kind="any"; query="${cur#,}" ;;
    *,,,) kind="any"; query="${cur%,,,}," ;;
    *,,f) kind="file"; query="${cur%,,f}," ;;
    *,,d) kind="dir"; query="${cur%,,d}," ;;
    *,,) kind="any"; query="${cur%,,}," ;;
    *) return 1 ;;
  esac

  local waymark_limit="${WAYMARK_COMPLETION_LIMIT:-1}"
  if [[ "$waymark_limit" != <-> ]] || (( waymark_limit < 1 )); then
    waymark_limit=1
  fi

  local -a query_tokens
  local waymark_token
  local waymark_trailing_comma=0
  [[ "$query" == *, ]] && waymark_trailing_comma=1
  for waymark_token in "${(@s:,:)query}"; do
    [[ -n "$waymark_token" ]] && query_tokens+=("$waymark_token")
  done
  (( waymark_trailing_comma )) && query_tokens+=("")

  local waymark_output
  waymark_output="$(command waymark query --kind "$kind" --limit "$waymark_limit" -- "$query_tokens[@]" 2>/dev/null)" || return 1
  [[ -n "$waymark_output" ]] || return 1

  local -a matches
  matches=("${(@f)waymark_output}")
  (( ${#matches} )) || return 1
  if (( ${#matches} > 1 && $+compstate )); then
    compstate[insert]=menu
    compstate[list]=list
  fi
  compadd -U -V waymark -- "$matches[@]"
}

autoload -Uz add-zsh-hook
add-zsh-hook chpwd _waymark_chpwd
add-zsh-hook preexec _waymark_preexec

_waymark_install_completion() {
  emulate -L zsh

  if (( $+functions[compdef] )); then
    compdef _waymark_comma_complete waymark z zz f v vv d a s sf sd
  fi

  local -a waymark_completers
  if ! zstyle -a ':completion:*' completer waymark_completers; then
    waymark_completers=(_complete)
  fi

  if (( ${waymark_completers[(Ie)_waymark_comma_complete]} == 0 )); then
    zstyle ':completion:*' completer _waymark_comma_complete "${waymark_completers[@]}"
  fi
}

_waymark_install_completion
"#
}
