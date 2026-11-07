import type { Linter } from 'eslint';
import type { ConfigOptions } from './index.js';

export interface ReactWebConfigOptions extends ConfigOptions {
  readonly domFiles?: string[];
}

export declare function reactWeb(options?: ReactWebConfigOptions): Linter.Config[];
