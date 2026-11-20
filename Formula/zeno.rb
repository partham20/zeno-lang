# Zeno, for Homebrew. This file is written by tools/package.sh when a
# version is released: change that, not this.
#
#     brew tap zeno-src/zeno-lang https://github.com/zeno-src/zeno-lang
#     brew install zeno-src/zeno-lang/zeno
class Zeno < Formula
  desc "Small, readable, statically typed language for firmware"
  homepage "https://zeno-lang.org"
  version "0.8.0"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/zeno-src/zeno-lang/releases/download/v0.8.0/zeno-0.8.0-macos-arm64.tar.gz"
      sha256 "4dac6bc12db09e251dba2fe473c1a3a76282db66e1dd17f7498c1b81e6c07b17"
    end
    on_intel do
      url "https://github.com/zeno-src/zeno-lang/releases/download/v0.8.0/zeno-0.8.0-macos-x86_64.tar.gz"
      sha256 "fe924d82c2704a9e1e1c94738ccfd42bc2eea9befe743393eed703de7c565991"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/zeno-src/zeno-lang/releases/download/v0.8.0/zeno-0.8.0-linux-arm64.tar.gz"
      sha256 "34eb14e1461a525ce452f4e0fddbb778c605c49c3cb8164c14ab38e07a013a72"
    end
    on_intel do
      url "https://github.com/zeno-src/zeno-lang/releases/download/v0.8.0/zeno-0.8.0-linux-x86_64.tar.gz"
      sha256 "5ca168e629e436c174bfe60cbaa08ae8bc24c093cfba13992aaa83ec1ff4f1f6"
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
