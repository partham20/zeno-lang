# Zeno, for Homebrew. This file is written by tools/package.sh when a
# version is released: change that, not this.
#
#     brew tap partham20/zeno-lang https://github.com/partham20/zeno-lang
#     brew install partham20/zeno-lang/zeno
class Zeno < Formula
  desc "Small, readable, statically typed language for firmware"
  homepage "https://zeno-lang.org"
  version "0.5.0"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/partham20/zeno-lang/releases/download/v0.5.0/zeno-0.5.0-macos-arm64.tar.gz"
      sha256 "48d059ccfcf22b5ae0a0fe7c97deb11181b5367eff7378d0b7c113e6a383d603"
    end
    on_intel do
      url "https://github.com/partham20/zeno-lang/releases/download/v0.5.0/zeno-0.5.0-macos-x86_64.tar.gz"
      sha256 "5c9250dc84071cce82a2400a224ec32641c9b21eeb16794c919afae2a49c7fb3"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/partham20/zeno-lang/releases/download/v0.5.0/zeno-0.5.0-linux-arm64.tar.gz"
      sha256 "177124497bb35d39ed86e02eae33a79f0446f81d67098a7f174a50b5524ff920"
    end
    on_intel do
      url "https://github.com/partham20/zeno-lang/releases/download/v0.5.0/zeno-0.5.0-linux-x86_64.tar.gz"
      sha256 "5c791bf9c61a4723c298e75e99c70cf07ee1b0d76df120464a3ef1bce8e35193"
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
