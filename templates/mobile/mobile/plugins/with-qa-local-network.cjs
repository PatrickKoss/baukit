const { withAndroidManifest, withInfoPlist } = require('expo/config-plugins');

module.exports = function withQaLocalNetwork(config) {
  const androidConfig = withAndroidManifest(config, (mod) => {
    const application = mod.modResults.manifest.application?.[0]?.$;
    if (!application) {
      throw new Error('Android application manifest entry is missing');
    }
    application['android:usesCleartextTraffic'] = 'true';
    return mod;
  });

  return withInfoPlist(androidConfig, (mod) => {
    mod.modResults.NSAppTransportSecurity = {
      ...mod.modResults.NSAppTransportSecurity,
      NSAllowsLocalNetworking: true,
    };
    return mod;
  });
};
