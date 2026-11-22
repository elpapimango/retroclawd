; P08: what the cartridge area shows beyond the end of the ROM. The ROM is padded with a pattern (byte = 4 KB block number);
; the bytes at the start of each tested address say which block of the ROM answered, or what an open bus returns.
; Result at RAM offset 0: the words read at $080000, $0C0000, $100000, $180000, $1FFFFE, $200000, $200100, $300000, $3FFFFE
; and bytes at $080001, $200001, $3FFFFF, then the same words again as longs.
    .org $200
    lea     $FF0000,a2
    move.w  $080000,(a2)+
    move.w  $0C0000,(a2)+
    move.w  $100000,(a2)+
    move.w  $180000,(a2)+
    move.w  $1FFFFE,(a2)+
    move.w  $200000,(a2)+
    move.w  $200100,(a2)+
    move.w  $300000,(a2)+
    move.w  $3FFFFE,(a2)+
    move.b  $080001,(a2)+
    move.b  $200001,(a2)+
    move.b  $3FFFFF,(a2)+
    move.b  #0,(a2)+
forever:
    bra.s   forever
