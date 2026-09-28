class Reactor < Formula
  desc "CLI for the Reactor backend"
  homepage "https://github.com/Reactor/reactor"
  url "https://github.com/Reactor/reactor/archive/refs/tags/v1.26.09-beta.1.tar.gz"
  # Filled once that tag's archive exists on GitHub. This beta does not publish a bottle.
  sha256 "0000000000000000000000000000000000000000000000000000000000000000"
  license "BUSL-1.1"
  version "1.26.09-beta.1"

  depends_on "rust" => :build

  def install
    system "cargo", "install", "--locked", "--path", "crates/reactor-cli", "--root", prefix
    mv bin/"reactor-cli", bin/"reactor"
  end

  test do
    assert_match "reactor", shell_output("#{bin}/reactor --help")
  end
end
