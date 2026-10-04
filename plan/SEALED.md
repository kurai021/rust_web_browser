# Framework SEAL — v1.2

- **Sealed on:** 2026-10-04 (UTC).
- **Status:** FROZEN. `plan/` is the source of truth for the implementation in `browser/`.
- **Sealed documents:** 16 (`00`–`15`).
- **v1.2 change vs v1.1:** full English translation — docs, filenames, code, comments and CLI strings. Also fixes a stray non-English token in doc 09. No scope change.
- **v1.1 change vs v1.0:** added doc 15 (SIMD/asm).

## Seal rules

1. No `plan/*.md` edit without bumping the framework version (v1.3, v1.4…) and re-sealing with new hashes + date + reason.
2. Code in `browser/` must cite the doc/section it implements. If the code needs anything outside the plan, propose the doc change first, version it, re-seal.
3. Per-phase evidence lives in `plan/evidence/phase-<n>/` (living appendix, not part of the seal).

## SHA-256 hashes (seal v1.2)

```
0c66194e82c22c6e420e563066b6cb3695697f9a9e98a3a2a069314a83754472  plan/00-index.md
15b3852e71d9497e84b3901627c285f569ea50ae79dc1a777ac125985fd4bf00  plan/15-asm-simd-optimization.md
3544944f0c155488639c67dd6aa118cce1ef1af8027ab896e2ceb313d142cd17  plan/08-javascript-engine.md
3ef532892c415273addd1a7e43543aab5a2ac5dc17c591da5695180f07bde75f  plan/07-layout-render-paint.md
49b5594f2a78792d40125c71693e2ec7587ff1e2d034feb751f730f929fec257  plan/04-network-https.md
61dfcee5f42ad7b919e6faefb9b9f65b9fd37919b70330e781141f8d2b4665d4  plan/05-html-parser.md
7503782cf7e0b29e110666836ed1898c709e4e202d60b0549666a089eb34b9b8  plan/09-dom-bom-webapis.md
893f7555520244a7f26bf12b62bbc1cb631853dd4e28ff7af03960bce9d375c7  plan/02-principles-constraints.md
8d29a61aec17bbb5d950957468ed26cf5043d1d8adc6d93a2b71de01e8448a17  plan/10-graphical-shell.md
8ff0450b0188b8712975fc12fc64d617004f558576233f01b30faeb2a618019e  plan/13-testing-compatibility.md
9ff32e174ff06df812f37a037bebca0cf6f8eb9c572a9380720bf91ed80e026c  plan/11-downloads-adblock-privacy.md
afa637e9167d6541e34311ff921486115af69d8d69fa2f73c938a7e04ebf03d6  plan/14-roadmap-phases.md
c20f6e1670e293ec6fc460fe2b5a414f6deca8c1b51ace671c3bf29016f9aaa1  plan/12-security.md
e472789ba398ec2e12ccc4530253df46e79fdaed1a51fcff8e6bfaf8a5d794de  plan/06-css.md
e5c7157fa0beb4f1e40b0f0f706629c1eb8761ae0d5490802ad59db6327043fc  plan/01-vision-scope.md
e7963e25b963505bf8206d2e447863983d8ea26f4c114252d31cb3d3796f9a41  plan/03-general-architecture.md
```

Verify with: `sha256sum plan/*.md | sort` (excluding this file).
