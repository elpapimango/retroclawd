; hello.s: a GEMDOS program for the Atari ST (S8P). `rcld build hello.layout.toml` makes hello.prg; EmuTOS runs it
; from the AUTO folder of a floppy the machine builds around it.
;
; GEMDOS function 9, Cconws, writes a string to the console: the string's address, then the function number, go on
; the stack and `trap #1` calls GEMDOS. The caller removes 6 bytes (a long and a word).

    .text
start:
    pea     msg(pc)             ; the address of the string (position independent: no fixup)
    move.w  #9,-(sp)            ; Cconws
    trap    #1
    addq.l  #6,sp
forever:
    bra.s   forever             ; keep the text on the screen

    .data
msg:
    dc.b    "HELLO FROM RCASM",13,10,0
