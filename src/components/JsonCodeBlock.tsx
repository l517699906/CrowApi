import { PrismLight as SyntaxHighlighter } from "react-syntax-highlighter";
import json from "react-syntax-highlighter/dist/esm/languages/prism/json";
import oneDark from "react-syntax-highlighter/dist/esm/styles/prism/one-dark";

SyntaxHighlighter.registerLanguage("json", json);

interface JsonCodeBlockProps {
    code: string;
}

export default function JsonCodeBlock({ code }: JsonCodeBlockProps) {
    return (
        <SyntaxHighlighter
            language="json"
            style={oneDark}
            wrapLongLines
            customStyle={{
                minHeight: "160px",
                maxHeight: "360px",
                margin: 0,
                overflow: "auto",
                padding: "16px",
                borderRadius: "7px",
                background: "var(--sidebar)",
            }}
            codeTagProps={{
                style: {
                    fontFamily: "var(--font-mono)",
                    fontSize: "11px",
                    lineHeight: 1.7,
                },
            }}
        >
            {code}
        </SyntaxHighlighter>
    );
}
