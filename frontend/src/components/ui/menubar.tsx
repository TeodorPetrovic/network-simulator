"use client"

import * as React from "react"
import { Menubar as MenubarPrimitive } from "radix-ui"
import { cn } from "cn"

function Menubar(props: React.ComponentProps<typeof MenubarPrimitive.Root>) {
  return <MenubarPrimitive.Root data-slot="menubar" className={cn("flex h-8 items-center gap-0.5 rounded-none border-0 bg-transparent p-0", props.className)} {...props} />
}
function MenubarMenu(props: React.ComponentProps<typeof MenubarPrimitive.Menu>) { return <MenubarPrimitive.Menu {...props} /> }
function MenubarTrigger({ className, ...props }: React.ComponentProps<typeof MenubarPrimitive.Trigger>) {
  return <MenubarPrimitive.Trigger data-slot="menubar-trigger" className={cn("rounded-sm px-2 py-1 text-sm outline-none hover:bg-accent focus:bg-accent data-[state=open]:bg-accent", className)} {...props} />
}
function MenubarContent({ className, ...props }: React.ComponentProps<typeof MenubarPrimitive.Content>) {
  return <MenubarPrimitive.Portal><MenubarPrimitive.Content data-slot="menubar-content" className={cn("z-50 min-w-52 overflow-hidden rounded-md border bg-popover p-1 text-popover-foreground shadow-md", className)} {...props} /></MenubarPrimitive.Portal>
}
function MenubarItem({ className, ...props }: React.ComponentProps<typeof MenubarPrimitive.Item>) {
  return <MenubarPrimitive.Item data-slot="menubar-item" className={cn("relative flex cursor-default select-none items-center rounded-sm px-2 py-1.5 text-sm outline-none focus:bg-accent focus:text-accent-foreground data-[disabled]:pointer-events-none data-[disabled]:opacity-50", className)} {...props} />
}
function MenubarSeparator({ className, ...props }: React.ComponentProps<typeof MenubarPrimitive.Separator>) {
  return <MenubarPrimitive.Separator data-slot="menubar-separator" className={cn("-mx-1 my-1 h-px bg-border", className)} {...props} />
}

export { Menubar, MenubarMenu, MenubarTrigger, MenubarContent, MenubarItem, MenubarSeparator }
