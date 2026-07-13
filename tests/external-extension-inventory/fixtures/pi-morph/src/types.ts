export interface MorphSettings {
  enabled: boolean;
  model: string;
  baseUrl: string;
  apiKeyProvider: string;
  maxFileBytes: number;
  maxOutputBytes: number;
  allowFullReplacement: boolean;
  showStatus: boolean;
  provider?: unknown;
  providerOptions?: unknown;
}

export type MorphEditParams = {
  target_filepath: string;
  instructions: string;
  code_edit: string;
};
