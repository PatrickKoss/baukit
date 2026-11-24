const fs = require('node:fs');
const path = require('node:path');
const { withAndroidManifest, withDangerousMod } = require('expo/config-plugins');

const BADGE_COLOR = '#F59E0B';
const BADGE_STROKE = '#FFFFFF';
const BADGE_SIZE_DP = 18;
const BADGE_STROKE_DP = 2;
const ICONS = [
  { attribute: 'android:icon', source: 'ic_launcher', badged: 'ic_launcher_variant' },
  { attribute: 'android:roundIcon', source: 'ic_launcher_round', badged: 'ic_launcher_round_variant' },
];

function badgedIcon(source) {
  return `<?xml version="1.0" encoding="utf-8"?>
<layer-list xmlns:android="http://schemas.android.com/apk/res/android">
  <item android:drawable="@mipmap/${source}" />
  <item android:gravity="top|end" android:width="${BADGE_SIZE_DP}dp" android:height="${BADGE_SIZE_DP}dp">
    <shape android:shape="oval">
      <solid android:color="${BADGE_COLOR}" />
      <stroke android:width="${BADGE_STROKE_DP}dp" android:color="${BADGE_STROKE}" />
    </shape>
  </item>
</layer-list>
`;
}

/** Layers a dot over the product's launcher icons, so a dev build is told apart on the home screen. */
function withVariantBadge(config) {
  const withDrawables = withDangerousMod(config, [
    'android',
    (mod) => {
      const drawables = path.join(mod.modRequest.platformProjectRoot, 'app', 'src', 'main', 'res', 'drawable');
      fs.mkdirSync(drawables, { recursive: true });
      for (const { source, badged } of ICONS) {
        fs.writeFileSync(path.join(drawables, `${badged}.xml`), badgedIcon(source));
      }
      return mod;
    },
  ]);
  return withAndroidManifest(withDrawables, (mod) => {
    const application = mod.modResults.manifest.application?.[0]?.$;
    if (!application) throw new Error('Android application manifest entry is missing');
    for (const { attribute, badged } of ICONS) application[attribute] = `@drawable/${badged}`;
    return mod;
  });
}

module.exports = withVariantBadge;
module.exports.badgedIcon = badgedIcon;
