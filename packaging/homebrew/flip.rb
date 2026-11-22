# typed: strict
# frozen_string_literal: true

cask "flip" do
  version "0.6.1"
  sha256 "75c72ba3dd8586ce53082cd268d8dbc49d62eccc81bbac3ad440e4a7e292e821"

  url "https://github.com/mehdizabo/FLIP-macOS/releases/download/v#{version}-macos/FLIP-#{version}-macOS-arm64.dmg"
  name "FLIP"
  desc "100% local Liquid Glass radial file converter and Create New File utility"
  homepage "https://github.com/thetinyfoundry/FLIP"

  depends_on arch: :arm64
  depends_on macos: ">= :sequoia"

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
