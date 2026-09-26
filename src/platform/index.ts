/** Host-specific behavior that is not a command: links, file selection and what the UI may
 *  offer at all. Components import from here rather than from a host module. */
import { host } from "@host";

export const capabilities = host.capabilities;
export const session = host.session;
export const operations = host.operations;
export const openExternal = host.openExternal;
export const pickDirectory = host.pickDirectory;
export const pickFile = host.pickFile;
export const selectBackup = host.selectBackup;
export const createBackup = host.createBackup;
