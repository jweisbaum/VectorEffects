import { Fragment, type ReactNode } from "react";

/**
 * A translated sentence with markup in it: each `{name}` placeholder becomes
 * the node given for it, so the sentence is one catalogue entry and each
 * language puts the code or the emphasis where its own word order wants it.
 */
export function rich(text: string, parts: Record<string, ReactNode>): ReactNode[] {
  return text.split(/(\{\w+\})/g).map((piece, index) => {
    const name = /^\{(\w+)\}$/.exec(piece)?.[1];
    return <Fragment key={index}>{name !== undefined && name in parts ? parts[name] : piece}</Fragment>;
  });
}
