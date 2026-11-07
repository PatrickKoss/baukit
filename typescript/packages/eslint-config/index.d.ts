import type { Linter } from 'eslint';

export interface ConfigOptions {
  readonly files?: string[];
}

export interface ScriptConfigOptions extends ConfigOptions {
  readonly globals?: Linter.Globals;
}

export declare const sourceFiles: string[];
export declare const testFiles: string[];
export declare const scriptFiles: string[];
export declare const baseRules: Linter.RulesRecord;
export declare const testRelaxedRules: Linter.RulesRecord;
export declare function base(options?: ConfigOptions): Linter.Config[];
export declare function react(options?: ConfigOptions): Linter.Config[];
export declare function scripts(options?: ScriptConfigOptions): Linter.Config[];
