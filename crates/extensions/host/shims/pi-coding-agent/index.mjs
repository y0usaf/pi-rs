import { component, stub } from "../../../common.mjs";

export function defineTool(tool) {
	return tool;
}

export function getAgentDir() {
	return globalThis.__piExtensionAgentDir;
}

export const createAgentSession = stub("createAgentSession");
export const createCodemodeExtension = stub("createCodemodeExtension");
export const SessionManager = component("SessionManager");
export const SettingsManager = component("SettingsManager");
export const DefaultResourceLoader = component("DefaultResourceLoader");
export const AssistantMessageComponent = component("AssistantMessageComponent");
export const CustomMessageComponent = component("CustomMessageComponent");
export const ToolExecutionComponent = component("ToolExecutionComponent");
export const UserMessageComponent = component("UserMessageComponent");
export const DynamicBorder = component("DynamicBorder");
export const keyHint = stub("keyHint");
export const rawKeyHint = stub("rawKeyHint");
