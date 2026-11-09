# Partner Center: submit Screentime for Windows

Everything to paste is in this folder: `listing.md`, `age-rating.md`, `runfulltrust.md`, `screenshots/`.
The file to upload is `store/out/Screentime_1.0.0.0_x64.msix` (unsigned; Microsoft signs it).

1. Go to https://partner.microsoft.com/dashboard > **Apps and games** > **Screentime for Windows** > **Start your submission**.
2. **Pricing and availability**
   - Markets: all markets (the default).
   - Visibility: Public audience, discoverable in the Store.
   - Pricing: **Free**. No free trial. No sales.
   - Release: "As soon as possible after it passes certification" (or pick a date).
   - Save.
3. **Properties**
   - Category: **Productivity**. Subcategory: none.
   - Privacy policy URL: `https://github.com/StarKnightt/screentime/blob/store/PRIVACY.md`
   - Website: `https://github.com/StarKnightt/screentime`
   - Support contact info: `https://github.com/StarKnightt/screentime/issues`
   - "Does this product access, collect, or transmit personal information?": **Yes** (it stores app usage on the PC; the privacy policy explains it stays there).
   - Product declarations: leave all unchecked, except check "This app was tested to meet accessibility guidelines" only if you want to claim it (not tested formally, so leave it unchecked).
   - System requirements: leave the defaults (Minimum hardware: none). Memory note optional: "Under 10 MB of memory".
   - Save.
4. **Age ratings**: Start the IARC questionnaire and answer it from `age-rating.md` (category "All other app types", every content question No). Save the rating.
5. **Packages**
   - Drag in `store/out/Screentime_1.0.0.0_x64.msix`.
   - Wait for the package to validate. Check it shows: Version 1.0.0.0, Architecture x64, device family Windows.Desktop, minimum version 10.0.22000.0, and capability runFullTrust.
   - Device family availability: **Windows 10/11 Desktop** checked; everything else unchecked.
   - Save.
6. **Store listings** > **English (United States)**
   - Description, What's new, Product features, Short description, Search terms, Copyright, Additional license terms: copy from `listing.md`.
   - Screenshots > Desktop: upload `screenshots/1-...png` to `5-...png` in order. Captions optional (the headline in each).
   - Store logos and trailers: optional, skip.
   - Save.
7. **Submission options**
   - Publishing hold options: as you prefer.
   - Notes for certification: paste the "Notes for certification" block from `runfulltrust.md`.
   - Restricted capabilities: paste the runFullTrust justification from `runfulltrust.md`.
   - Save.
8. Back on the submission overview, every section should say **Complete**. Click **Submit to the Store**.
9. Certification usually takes from a few hours to three business days. Partner Center emails you the result; if it fails, the report names the policy and section, and nothing goes live until you resubmit.

If certification says the app's display name must match the reserved name: change `DisplayName="Screentime"` on `uap:VisualElements` in `store/AppxManifest.xml` to `Screentime for Windows`, run `store\build.ps1` again and upload the new .msix.
