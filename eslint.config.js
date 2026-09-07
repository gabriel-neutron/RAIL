import js from '@eslint/js';
import globals from 'globals';
import tseslint from 'typescript-eslint';
import reactHooks from 'eslint-plugin-react-hooks';
import reactRefresh from 'eslint-plugin-react-refresh';

export default tseslint.config(
  {
    ignores: [
      'dist',
      'node_modules',
      'src-tauri',
      // Generated output; the codegen script is the source of truth.
      'src/ipc/generated',
      // Standalone slide generator, not part of the app build.
      'RAIL_slides',
    ],
  },
  js.configs.recommended,
  {
    // Type-aware rules only apply to the TS sources; config/build .js files
    // are outside the tsconfig project and would fail to resolve types.
    files: ['**/*.{ts,tsx}'],
    extends: [...tseslint.configs.recommendedTypeChecked],
    languageOptions: {
      globals: globals.browser,
      parserOptions: {
        projectService: true,
        tsconfigRootDir: import.meta.dirname,
      },
    },
    plugins: {
      'react-hooks': reactHooks,
      'react-refresh': reactRefresh,
    },
    rules: {
      ...reactHooks.configs.recommended.rules,

      // React Compiler ergonomics rules. 7 pre-existing violations across
      // AudioControls / Transport / FrequencyControl / PpmControl / Scanner —
      // each needs a real component refactor, so they warn rather than block.
      // TODO: fix those, then raise both to 'error'.
      'react-hooks/refs': 'warn',
      'react-hooks/set-state-in-effect': 'warn',
      // typescript-eslint's equivalents handle these; the core rules produce
      // false positives on TS (types, ambient DOM globals, enums).
      'no-undef': 'off',
      'no-unused-vars': 'off',
      'react-refresh/only-export-components': ['warn', { allowConstantExport: true }],

      // CLAUDE.md: "No `any` in TypeScript". tsc's `strict` does not catch
      // explicit `any`, so it is enforced here.
      '@typescript-eslint/no-explicit-any': 'error',
      '@typescript-eslint/no-unsafe-assignment': 'error',
      '@typescript-eslint/no-unsafe-member-access': 'error',
      '@typescript-eslint/no-unsafe-call': 'error',
      '@typescript-eslint/no-unsafe-return': 'error',

      // Unawaited promises silently swallow IPC failures.
      '@typescript-eslint/no-floating-promises': 'error',
      '@typescript-eslint/no-misused-promises': 'error',
    },
  },
  {
    // Node-side build/codegen scripts.
    files: ['**/*.mjs', 'scripts/**/*.js'],
    languageOptions: { globals: globals.node },
  },
  {
    files: ['**/*.test.{ts,tsx}'],
    rules: { '@typescript-eslint/no-non-null-assertion': 'off' },
  },
);
