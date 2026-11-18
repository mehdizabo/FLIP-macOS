cask "flip" do
  version "0.1.0"
  sha256 "7aa65cea17524e73db9edd3e1771eb43531baba0005c51d1199ea96380674668"

  url "https://github.com/thetinyfoundry/FLIP/releases/download/v#{version}/FLIP-#{version}-macOS-arm64.dmg"
  name "FLIP"
  desc "100% local Liquid Glass radial file converter and Create New File utility"
  homepage "https://github.com/thetinyfoundry/FLIP"

  depends_on macos: ">= :sequoia"
  depends_on arch: :arm64

  app "FLIP.app"
  binary "#{appdir}/FLIP.app/Contents/MacOS/flip"

  zap trash: [
    "~/Library/Application Support/FLIP",
    "~/Library/Caches/com.thetinyfoundry.flip.mac",
    "~/Library/LaunchAgents/com.thetinyfoundry.flip.mac.plist",
    "~/Library/Preferences/com.thetinyfoundry.flip.mac.plist",
    "~/Library/Services/Convert with FLIP.workflow",
    "~/Library/Services/New File with FLIP.workflow",
  ]
end
