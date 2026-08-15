declare module "react-syntax-highlighter" {
    import type { ComponentType, CSSProperties, HTMLAttributes, ReactNode } from "react";

    interface SyntaxHighlighterProps {
        children: ReactNode;
        className?: string;
        codeTagProps?: HTMLAttributes<HTMLElement>;
        customStyle?: CSSProperties;
        language?: string;
        showLineNumbers?: boolean;
        style?: Record<string, CSSProperties>;
        wrapLongLines?: boolean;
    }

    type SyntaxHighlighterComponent = ComponentType<SyntaxHighlighterProps> & {
        registerLanguage: (name: string, language: unknown) => void;
    };

    export const PrismLight: SyntaxHighlighterComponent;
}

declare module "react-syntax-highlighter/dist/esm/styles/prism/one-dark" {
    import type { CSSProperties } from "react";

    const style: Record<string, CSSProperties>;
    export default style;
}

declare module "react-syntax-highlighter/dist/esm/languages/prism/json" {
    const language: unknown;
    export default language;
}
