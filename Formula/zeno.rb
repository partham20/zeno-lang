# Zeno, for Homebrew. This file is written by tools/package.sh when a
# version is released: change that, not this.
#
#     brew tap partham20/zeno-lang https://github.com/partham20/zeno-lang
#     brew install partham20/zeno-lang/zeno
class Zeno < Formula
  desc "Small, readable, statically typed language for firmware"
  homepage "https://zeno-lang.org"
  version "0.1.0"

  on_macos do
    on_arm do
      url "https://github.com/partham20/zeno-lang/releases/download/v0.1.0/zeno-0.1.0-macos-arm64.tar.gz"
      sha256 "b552aa8f1e23c72255898559217e6a72d86851b1db970a485a37a21c9d2b4720"
    end
    on_intel do
      url "https://github.com/partham20/zeno-lang/releases/download/v0.1.0/zeno-0.1.0-macos-x86_64.tar.gz"
      sha256 "545fcab7f618b4a4a60d2ab801a7434c5212c4f91ebac0db8121d9a92e645419"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/partham20/zeno-lang/releases/download/v0.1.0/zeno-0.1.0-linux-arm64.tar.gz"
      sha256 "535ff05612746d6d9f7b49aedfe48f3d8f32df9d9e1cfdf58b536257ee3bdf66"
    end
    on_intel do
      url "https://github.com/partham20/zeno-lang/releases/download/v0.1.0/zeno-0.1.0-linux-x86_64.tar.gz"
      sha256 "b3442b311d0357cb4ac50e554d3b751b781d3620720f42308451219cc75563ff"
    end
  end

  def install
    bin.install "zeno"
    # Zeno built for a browser, which `zeno build --web` looks for
    # beside `zeno`.
    bin.install "zeno.wasm" if File.exist?("zeno.wasm")
    doc.install "README.md", "docs"
    pkgshare.install "examples", "editors"
  end

  def caveats
    <<~EOS
      `zeno build` hands the C it writes to a C compiler, and `zeno fetch`
      gets packages with git. On macOS both come with Apple's command
      line tools: xcode-select --install
      The examples are in #{opt_pkgshare}/examples
    EOS
  end

  test do
    assert_equal "zeno #{version}", shell_output("#{bin}/zeno --version").strip
    assert_equal "Hello, Zeno!", shell_output("#{bin}/zeno run #{pkgshare}/examples/hello.zeno").strip
  end
end
