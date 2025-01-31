#!/usr/bin/env bash
set +e  # Disable immediate exit on error

crates=($(cargo metadata --format-version=1 --no-deps | jq -r '.packages[].name' | sort))

exclude_crates=(
  engine
  scroll-bridge
  scroll-wire
  scroll-network
)

contains() {
  local ex="$1[@]"
  local input=$2

  for ignore in "${!ex}";
  do
    if [ "$ignore" = "$input" ];
    then
      return 0
    fi
  done
  return 1
}

results=()
any_failed=0

for crate in "${crates[@]}";
do
  if contains exclude_crates "$crate";
  then
    results+=("⏭️ $crate")
    continue
  fi

  cmd="cargo +stable build -p $crate --target wasm-unknown-unknown --no-default-features"

  set +e  # Disable immediate exit on error
  # Run the command and capture the return code
  $cmd
  ret_code=$?
  set -e  # Re-enable immediate exit on error

  if [ $ret_code -eq 0 ];
  then
    results+=("✅ $crate")
  else
    results+=("❌ $crate")
    any_failed=1
  fi
done

IFS=$'\n' sorted=$(sort <<< "${results[*]}")
unset IFS

printf "Build results: \n"
for result in "${sorted[@]}";
do
  echo "${result}"
done

exit $any_failed
