export type ToolSummaryAdapter = {
  summarizeArgs?: (args: any) => string | undefined;
  summarizeResult?: (result: any) => string | undefined;
};

export type ToolSummaryRegistry = Record<string, ToolSummaryAdapter>;
