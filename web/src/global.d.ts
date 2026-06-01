export {};

declare global {
	interface Window {
		__sendPrompt?: (text: string) => Promise<void>;
		__stopPrompt?: () => void;
		__steerPrompt?: (text: string) => Promise<void>;
		__resetAgent?: () => void;
	}
}
