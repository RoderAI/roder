class Roder < Formula
  desc "Rust-native TUI coding agent and event-driven agent harness"
  homepage "https://github.com/PandelisZ/gode"
  url "https://github.com/PandelisZ/gode.git", branch: "main"
  version "0.0.0"
  head "https://github.com/PandelisZ/gode.git", branch: "main"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args(path: "crates/roder-cli")
  end

  test do
    assert_match "codex:", shell_output("#{bin}/roder auth status")
  end
end
