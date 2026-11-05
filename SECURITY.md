# Security

Native modules execute with the browser's privileges. Only official Developer ID signed and Apple-notarized packages should be installed. Checksums detect release corruption; signing and notarization establish executable trust.

Do not publish signing certificates, private keys, notary credentials or profile data in issues. Report a suspected vulnerability privately to the repository owner through GitHub. Credential rotation must keep the configured official Team ID; a publisher-team change requires a reviewed browser trust update.
