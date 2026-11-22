; P03: byte accesses to the VDP ports. What a byte write to the data and control ports does, and what byte reads return.
; Result at RAM offset 0 (bytes):
;   $00: VRAM $0000..$000F after byte writes to the data port ($C00000 = $12, $C00001 = $34, then $C00000 = $56 again)
;   $10: VRAM $0020..$002F after the control port was set up by byte writes ($8F,$04 to $C00004/5, then $40,$20 / $00,$00)
;   $20: bytes read from $C00005 (status low), $C00001 (data), $C00009 (HV low) -- not compared across emulators
    .org $200
    lea     $C00000,a0
    lea     $C00004,a1
    lea     $FF0000,a2
    move.w  #$8004,(a1)
    move.w  #$8104,(a1)
    move.w  #$8F02,(a1)
    move.l  #$40000000,(a1)
    move.b  #$12,$C00000
    move.b  #$34,$C00001
    move.b  #$56,$C00000
    move.l  #$00000000,(a1)
    moveq   #7,d0
r1: move.w  (a0),(a2)+
    dbf     d0,r1
    ; control port by bytes: register 15 (increment) = 4 through a byte pair, then a VRAM write command by bytes
    move.b  #$8F,$C00004
    move.b  #$04,$C00005
    move.b  #$40,$C00004
    move.b  #$20,$C00005
    move.b  #$00,$C00004
    move.b  #$00,$C00005
    move.w  #$AAAA,(a0)
    move.w  #$BBBB,(a0)
    move.w  #$CCCC,(a0)
    move.w  #$8F02,(a1)
    move.l  #$00200000,(a1)
    moveq   #7,d0
r2: move.w  (a0),(a2)+
    dbf     d0,r2
    move.b  $C00005,(a2)+
    move.b  $C00001,(a2)+
    move.b  $C00009,(a2)+
forever:
    bra.s   forever
