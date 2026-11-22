; P02: VDP memory access through the data port: VRAM word writes at an even and an odd address, auto-increment 1 and 2,
; CRAM and VSRAM writes and reads (which bits survive, what the end of each memory does), the status register afterwards.
; Result at RAM offset 0 (words, big-endian):
;   $00: VRAM $0000..$000E   $10: VRAM $0100..$010E   $20: VRAM $0200..$020E
;   $30: CRAM idx 0..7       $40: CRAM $7C,$7E,$00(wrap) then idx 0   $48: VSRAM idx 0..7
;   $58: VSRAM at $4E (idx 39), $50 (idx 40), $7E (idx 63)   $5E: status
    .org $200
    lea     $C00000,a0
    lea     $C00004,a1
    lea     $FF0000,a2
    move.w  #$8004,(a1)
    move.w  #$8104,(a1)
    move.w  #$8F02,(a1)
    move.l  #$40000000,(a1)
    move.w  #$1234,(a0)
    move.w  #$5678,(a0)
    move.l  #$41010000,(a1)
    move.w  #$ABCD,(a0)
    move.w  #$EF01,(a0)
    move.w  #$8F01,(a1)
    move.l  #$42000000,(a1)
    move.w  #$1122,(a0)
    move.w  #$3344,(a0)
    move.w  #$8F02,(a1)
    move.l  #$00000000,(a1)
    moveq   #7,d0
r1: move.w  (a0),(a2)+
    dbf     d0,r1
    move.l  #$01000000,(a1)
    moveq   #7,d0
r2: move.w  (a0),(a2)+
    dbf     d0,r2
    move.l  #$02000000,(a1)
    moveq   #7,d0
r3: move.w  (a0),(a2)+
    dbf     d0,r3
    ; CRAM
    move.l  #$C0000000,(a1)
    move.w  #$0EEE,(a0)
    move.w  #$0000,(a0)
    move.w  #$0123,(a0)
    move.w  #$0FFF,(a0)
    move.w  #$0444,(a0)
    move.w  #$0888,(a0)
    move.w  #$0E00,(a0)
    move.w  #$00E0,(a0)
    move.l  #$C07C0000,(a1)
    move.w  #$0111,(a0)
    move.w  #$0222,(a0)
    move.w  #$0333,(a0)
    move.l  #$00000020,(a1)
    moveq   #7,d0
r4: move.w  (a0),(a2)+
    dbf     d0,r4
    move.l  #$007C0020,(a1)
    move.w  (a0),(a2)+
    move.w  (a0),(a2)+
    move.w  (a0),(a2)+
    move.l  #$00000020,(a1)
    move.w  (a0),(a2)+
    ; VSRAM
    move.l  #$40000010,(a1)
    move.w  #$0001,(a0)
    move.w  #$03FF,(a0)
    move.w  #$0400,(a0)
    move.w  #$FFFF,(a0)
    move.w  #$AAAA,(a0)
    move.w  #$5555,(a0)
    move.w  #$0123,(a0)
    move.w  #$0321,(a0)
    move.l  #$00000010,(a1)
    moveq   #7,d0
r5: move.w  (a0),(a2)+
    dbf     d0,r5
    move.l  #$404E0010,(a1)
    move.w  #$0777,(a0)
    move.w  #$0666,(a0)
    move.l  #$407E0010,(a1)
    move.w  #$0555,(a0)
    move.l  #$004E0010,(a1)
    move.w  (a0),(a2)+
    move.w  (a0),(a2)+
    move.l  #$007E0010,(a1)
    move.w  (a0),(a2)+
    move.w  (a1),(a2)+
forever:
    bra.s   forever
