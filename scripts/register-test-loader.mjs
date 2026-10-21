// The supported Node 22 runtime may be compiled without native TS stripping.
// Only the test process installs this loader; packaged worker code is bundled.
import { register } from 'node:module';
register('./ts-test-loader.mjs', import.meta.url);
