// MP-08 / MP-09 / MP-10 / MP-11. Dedicated client-view browser only.
// An ordinary Web page cannot enforce process launch flags. AV4/AV5 must own
// this launch or fail closed; flags are an additional layer, never a CSP substitute.
export const clientViewChromiumArgs = Object.freeze([
  '--no-proxy-server',
  '--host-resolver-rules=MAP * ~NOTFOUND, EXCLUDE localhost, EXCLUDE 127.0.0.1, EXCLUDE [::1]',
  '--disable-background-networking',
  '--disable-component-update',
  '--disable-component-extensions-with-background-pages',
  '--disable-domain-reliability',
  '--disable-sync',
  '--disable-default-apps',
  '--disable-extensions',
  '--disable-breakpad',
  '--disable-client-side-phishing-detection',
  '--safebrowsing-disable-auto-update',
  '--no-pings',
  '--no-first-run',
  '--no-default-browser-check',
  '--disable-features=OptimizationHints,OptimizationGuideModelDownloading,OptimizationTargetPrediction,MediaRouter,AutofillServerCommunication,CertificateTransparencyComponentUpdater,InterestFeedContentSuggestions,Translate,Prerender2,SpeculationRulesPrefetch,PreloadingHeuristicsMLModel',
]);

// Chrome155 removed the old disable-preconnect switch. Disable prediction in
// the fresh isolated profile as well as the launch features.
export const clientViewChromiumPreferences = Object.freeze({
  net: { network_prediction_options: 2 },
  safebrowsing: { enabled: false, enhanced: false },
  search: { suggest_enabled: false },
  dns_over_https: { mode: 'off' },
});
