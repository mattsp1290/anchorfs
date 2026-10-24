# Sourced helper: sha256_of <file> prints the lowercase hex SHA-256 of a file
# (sha256sum where present, else shasum -a 256). Shared by the release build and
# the cluster scripts.
sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}
