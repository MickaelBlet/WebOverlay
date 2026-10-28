# Desktop builds (see Dockerfile): `docker buildx bake [linux|windows]`, both in parallel by default;
# `docker buildx bake check` runs fmt, clippy and tests.
variable "FAST" {
  default = ""
}

group "default" {
  targets = ["linux", "windows"]
}

target "_common" {
  dockerfile = "Dockerfile"
  platforms  = ["linux/amd64"]
  args       = { FAST = FAST }
}

target "linux" {
  inherits = ["_common"]
  target   = "linux-out"
  output   = ["type=local,dest=dist-tauri/linux"]
}

target "windows" {
  inherits = ["_common"]
  target   = "windows-out"
  output   = ["type=local,dest=dist-tauri/windows"]
}

target "check" {
  inherits = ["_common"]
  target   = "check"
  output   = ["type=cacheonly"]
}
