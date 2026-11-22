; P05: the Z80 from the 68000's side and from its own side.
; 68000 side: the bus request and reset registers (power-on, request, grant), Z80 RAM word/byte access, the YM2612 ports.
; Z80 side (p05_z80.z80.s, copied to Z80 RAM): RAM mirror, YM2612 mirrors, the bank register and the 68000 window, the VDP.
; Result at RAM offset 0:
;   $00 words: $A11100 and $A11200 at power on
;   $04 bytes: $A11100, $A11101 at power on
;   $06 words: $A11100 and $A11200 after a request and reset release, then bytes $A11100 / $A11101
;   $0C: Z80 RAM test: words at $A00000, $A00002, $A00004, bytes at $A00006, $A00007, $A00001
;   $14: bytes $A04000, $A04001, $A04002, $A04003 as the 68000 reads them
;   $20: 32 bytes of Z80 RAM $1F00-$1F1F after the Z80 program ran, $40: the next 32 bytes
    .org $200
    lea     $FF0000,a2
    move.w  $A11100,(a2)+
    move.w  $A11200,(a2)+
    move.b  $A11100,(a2)+
    move.b  $A11101,(a2)+
    ; the documented sequence: reset, request the bus, release reset
    move.w  #$0000,$A11200
    move.w  #$0100,$A11100
    move.w  #$0100,$A11200
    move.w  #2000,d0
grant1:
    btst    #0,$A11100
    beq.s   granted1
    dbf     d0,grant1
granted1:
    move.w  $A11100,(a2)+
    move.w  $A11200,(a2)+
    move.b  $A11100,(a2)+
    move.b  $A11101,(a2)+
    ; Z80 RAM from the 68000
    move.b  #$12,$A00000
    move.w  #$3456,$A00002
    move.w  #$789A,$A00004
    move.b  #$BC,$A00007
    move.w  $A00000,(a2)+
    move.w  $A00002,(a2)+
    move.w  $A00004,(a2)+
    move.b  $A00006,(a2)+
    move.b  $A00007,(a2)+
    move.b  $A00001,(a2)+
    move.b  $A04000,(a2)+
    move.b  $A04001,(a2)+
    move.b  $A04002,(a2)+
    move.b  $A04003,(a2)+
    ; data the Z80 reads through its window
    move.b  #$5A,$FF8000
    move.b  #$A5,$FF8001
    ; copy the Z80 program
    lea     z80code(pc),a0
    lea     $A00000,a1
    move.w  #(z80end-z80code)-1,d0
cp: move.b  (a0)+,(a1)+
    dbf     d0,cp
    ; clear the result area of the Z80 RAM
    lea     $A01F00,a1
    moveq   #63,d0
clr:
    clr.b   (a1)+
    dbf     d0,clr
    ; run it: reset, release the reset, release the bus
    move.w  #$0000,$A11200
    moveq   #50,d0
rd: dbf     d0,rd
    move.w  #$0100,$A11200
    move.w  #$0000,$A11100
    ; wait
    move.l  #60000,d0
wt: subq.l  #1,d0
    bne.s   wt
    ; stop the Z80 and fetch what it left
    move.w  #$0100,$A11100
    move.w  #2000,d0
grant2:
    btst    #0,$A11100
    beq.s   granted2
    dbf     d0,grant2
granted2:
    lea     $FF0020,a2
    lea     $A01F00,a1
    moveq   #63,d0
get:
    move.b  (a1)+,(a2)+
    dbf     d0,get
forever:
    bra.s   forever
z80code:
    .include "p05_z80.z80.inc"
z80end:
