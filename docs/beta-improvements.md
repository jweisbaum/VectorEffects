# Beta improvements — September 2026

Requested work, in implementation order:

- [x] Export undefined cells with GRIB bitmaps, preserving genuinely calm cells.
- [x] Hide missing files from recent projects.
- [x] Retry failed history imports from the bottom error panel.
- [x] Block this beta beginning January 1, 2027 using the system's local date.
- [x] Preserve pixel brush proportions in every supported map projection, including previews, erasing, and CPU/GPU rendering.
- [x] Carry only selected objects in the drag preview.
- [x] Add constant position motion from the selected frame to the next keyframe (or timeline end), with direction/speed inputs and overwrite confirmation.
- [x] Offer speed thresholds on every layer, applied to its final field contribution.
- [x] Bundle help text and real application screenshots behind the Help menu.
- [x] Prepare four-platform build/release workflows and a GitHub publishing plan.
- [x] Finish native regression/build checks and commit all requested work locally.

Projection is captured with pixel-created geometry. Changing the view later does not rewrite project geometry or exported data. The expiration boundary is local midnight on January 1, 2027; both the startup screen and native command handling enforce it. Image layers have no vector speed of their own, so their threshold is evaluated against the displayed vector field.


Verification on September 13, 2026:

- Frontend: 740 tests across 72 files, TypeScript check, production Vite build,
  and the offline asset/network check passed.
- Native Clippy with warnings denied and Rust formatting passed. The production
  executable built with `npm run build -- --no-bundle`, without WebDriver; its
  embedded asset names match the current frontend and all eight Help images.
- The workspace sweep completed with 1,085 passing tests, 13 existing ignored
  fixture/benchmark tests, and one incorrect new test fixture. That fixture
  requested a 200% gain while expecting doubled speed. After changing it to 100%,
  all 47 renderer evaluation tests passed. No implementation change was needed.
  The latest motion/selection/shape suites (60 tests) and cache tests (6) also
  passed. Including the added Metal threshold test, 1,087 distinct native tests
  were validated across the sweep and focused reruns.
- Metal fidelity: all six tests passed on the AMD Radeon Pro 5500M. Across
  30,658 comparisons, maximum speed error was 0.0159 m/s and maximum direction
  error was 0.008°. Sixty-two boundary/tangent ties used the existing exemptions.
  The explicit modified-layer threshold comparison also passed on Metal.
- ecCodes independently decoded six painted and six entirely missing messages.
  Painted east/north vectors and their zero components were correct; unpainted
  ocean was missing. Every empty message had 65,160 missing cells and zero
  defined values.
- Eight screenshots were generated from the running native app in isolated
  storage, then visually inspected. Screenshots show real map rendering,
  perimeter controls, pixel tool options, and the motion/import/settings/export
  dialogs. The Help menu event and topic navigation also have a frontend test. A live
  layout check confirmed a long download error leaves Retry visible and that
  clicking it runs the retry once.
- The release plan records the remaining GitHub authentication, hosted platform
  builds, signing, and installation checks; none are claimed to have run.
