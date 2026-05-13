class Gode < Formula
  desc "Go-native TUI coding agent and event-driven agent harness"
  homepage "https://github.com/PandelisZ/gode"
  url "https://github.com/PandelisZ/gode.git", branch: "main"
  version "0.0.0"
  head "https://github.com/PandelisZ/gode.git", branch: "main"

  depends_on "go" => :build

  def install
    ldflags = "-s -w -X main.version=#{version}"
    system "go", "build", "-trimpath", "-ldflags", ldflags, "-o", bin/"gode", "./cmd/gode"
  end

  test do
    assert_match "gode #{version}", shell_output("#{bin}/gode version")
  end
end
