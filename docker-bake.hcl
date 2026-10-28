# Desktop builds (see Dockerfile): `docker buildx bake` builds Linux and Windows in parallel.
group "default" {
  targets = ["tauri"]
}

target "tauri" {
  dockerfile = "Dockerfile"
  platforms  = ["linux/amd64"]
  target     = "tauri"
  output     = ["type=local,dest=dist-tauri"]
}
